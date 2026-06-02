use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use libp2p_identity::Keypair;
use rand_core::{OsRng, RngCore};
use secp256k1::SecretKey;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use thiserror::Error;
use zeroize::Zeroize;

const KEYSTORE_V1_AAD_PREFIX: &[u8] = b"ghal_bol.keystore.v1";

#[derive(Debug, Error)]
pub enum KeystoreError {
    #[error("keystore json: {0}")]
    Json(#[from] serde_json::Error),

    #[error("invalid keystore: {0}")]
    Invalid(&'static str),

    #[error("kdf parameters rejected")]
    KdfParams,

    #[error("kdf failed")]
    Kdf,

    #[error("decrypt failed (wrong password or corrupted data)")]
    Decrypt,
}

#[derive(Debug, Error)]
pub enum Libp2pIdentityError {
    #[error("libp2p secp256k1 secret: {0}")]
    SecretKey(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeystoreV1KdfParams {
    #[serde(with = "serde_bytes")]
    pub salt: Vec<u8>,
    pub m_cost_kib: u32,
    pub t_cost: u32,
    pub p_cost: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeystoreV1 {
    pub format: String,
    pub kdf: KeystoreV1KdfParams,
    #[serde(with = "serde_bytes")]
    pub identity_public_key: Vec<u8>,
    #[serde(with = "serde_bytes")]
    pub identity_nonce: Vec<u8>,
    #[serde(with = "serde_bytes")]
    pub identity_ciphertext: Vec<u8>,
}

/// Unlocked identity: one secp256k1 keypair (libp2p PeerId, sign, encrypt).
#[derive(Clone)]
pub struct DecryptedIdentity {
    keypair: Keypair,
    secret: SecretKey,
}

impl core::fmt::Debug for DecryptedIdentity {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DecryptedIdentity")
            .field("public_key", &"<secp256k1 compressed>")
            .finish()
    }
}

impl DecryptedIdentity {
    pub fn keypair(&self) -> &Keypair {
        &self.keypair
    }

    pub fn secp256k1_secret(&self) -> &SecretKey {
        &self.secret
    }

    /// 66-hex-char compressed secp256k1 public key (device identity).
    pub fn public_key_hex(&self) -> String {
        let pk = self
            .keypair
            .public()
            .try_into_secp256k1()
            .expect("secp256k1 keypair");
        crate::public_key_util::secp256k1_public_key_to_hex(&pk)
    }

    pub fn public_key_bytes(&self) -> Vec<u8> {
        self.keypair
            .public()
            .try_into_secp256k1()
            .expect("secp256k1 keypair")
            .to_bytes()
            .to_vec()
    }

    pub fn to_libp2p_keypair(&self) -> Result<Keypair, Libp2pIdentityError> {
        Ok(self.keypair.clone())
    }
}

fn derive_master_key(password: &[u8], kdf: &KeystoreV1KdfParams) -> Result<[u8; 32], KeystoreError> {
    if kdf.salt.len() < 16 {
        return Err(KeystoreError::Invalid("salt too short"));
    }
    let params = Params::new(kdf.m_cost_kib, kdf.t_cost, kdf.p_cost, Some(32))
        .map_err(|_| KeystoreError::KdfParams)?;
    let a2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut out = [0u8; 32];
    a2.hash_password_into(password, &kdf.salt, &mut out)
        .map_err(|_| KeystoreError::Kdf)?;
    Ok(out)
}

fn derive_aead_key(master: &[u8; 32], purpose: &'static [u8]) -> [u8; 32] {
    let hk = hkdf::Hkdf::<Sha256>::new(None, master);
    let mut okm = [0u8; 32];
    hk.expand(purpose, &mut okm)
        .expect("hkdf expand to 32 bytes cannot fail");
    okm
}

fn aad(purpose: &'static [u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(KEYSTORE_V1_AAD_PREFIX.len() + 1 + purpose.len());
    v.extend_from_slice(KEYSTORE_V1_AAD_PREFIX);
    v.push(b':');
    v.extend_from_slice(purpose);
    v
}

fn keypair_from_secret_bytes(sk: &[u8]) -> Result<(Keypair, SecretKey), KeystoreError> {
    if sk.len() != 32 {
        return Err(KeystoreError::Invalid("bad secret length"));
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(sk);
    let secret = SecretKey::from_byte_array(arr)
        .map_err(|_| KeystoreError::Invalid("invalid secp256k1 secret"))?;
    let mut sk_bytes = secret.secret_bytes();
    let libp2p_secret = libp2p_identity::secp256k1::SecretKey::try_from_bytes(&mut sk_bytes)
        .map_err(|_| KeystoreError::Invalid("libp2p secp256k1 secret"))?;
    let secp_kp = libp2p_identity::secp256k1::Keypair::from(libp2p_secret);
    let keypair = Keypair::from(secp_kp);
    Ok((keypair, secret))
}

/// Parse a 32-byte secp256k1 secret as 64 lowercase/uppercase hex digits (no `0x` prefix).
pub fn parse_secret_key_hex(secret_hex: &str) -> Result<[u8; 32], KeystoreError> {
    let s = secret_hex.trim();
    if s.len() != 64 {
        return Err(KeystoreError::Invalid("secret key hex must be 64 characters"));
    }
    let bytes = hex::decode(s).map_err(|_| KeystoreError::Invalid("invalid secret key hex"))?;
    let arr: [u8; 32] = bytes
        .try_into()
        .map_err(|_| KeystoreError::Invalid("secret key must be 32 bytes"))?;
    Ok(arr)
}

/// Build an encrypted keystore from an existing 32-byte secret (import path).
pub fn create_keystore_v1_from_secret(
    password: &str,
    secret: &[u8; 32],
    kdf_override: Option<KeystoreV1KdfParams>,
) -> Result<(KeystoreV1, DecryptedIdentity), KeystoreError> {
    let mut rng = OsRng;
    let (keypair, secret) = keypair_from_secret_bytes(secret)?;

    let kdf = if let Some(k) = kdf_override {
        k
    } else {
        let mut salt = vec![0u8; 16];
        rng.fill_bytes(&mut salt);
        KeystoreV1KdfParams {
            salt,
            m_cost_kib: 64 * 1024,
            t_cost: 2,
            p_cost: 1,
        }
    };

    let master = derive_master_key(password.as_bytes(), &kdf)?;
    let mut identity_key = derive_aead_key(&master, b"identity");
    let aead = ChaCha20Poly1305::new(Key::from_slice(&identity_key));
    let mut nonce = [0u8; 12];
    rng.fill_bytes(&mut nonce);
    let mut sk_plain = secret.secret_bytes();
    let ct = aead
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: &sk_plain,
                aad: &aad(b"identity"),
            },
        )
        .map_err(|_| KeystoreError::Invalid("encrypt identity failed"))?;
    sk_plain.zeroize();
    identity_key.zeroize();

    let keystore = KeystoreV1 {
        format: "keystore_v1".to_string(),
        kdf,
        identity_public_key: keypair.public().try_into_secp256k1().unwrap().to_bytes().to_vec(),
        identity_nonce: nonce.to_vec(),
        identity_ciphertext: ct,
    };
    Ok((keystore, DecryptedIdentity { keypair, secret }))
}

pub fn create_keystore_v1(
    password: &str,
    kdf_override: Option<KeystoreV1KdfParams>,
) -> Result<(KeystoreV1, DecryptedIdentity), KeystoreError> {
    let mut sk_bytes = [0u8; 32];
    OsRng.fill_bytes(&mut sk_bytes);
    let secret = SecretKey::from_byte_array(sk_bytes)
        .map_err(|_| KeystoreError::Invalid("generated secp256k1 secret"))?;
    create_keystore_v1_from_secret(password, &secret.secret_bytes(), kdf_override)
}

/// 64-char lowercase hex of the secp256k1 secret (caller must protect output).
pub fn secret_key_hex_from_identity(id: &DecryptedIdentity) -> String {
    hex::encode(id.secp256k1_secret().secret_bytes())
}

pub fn unlock_keystore_v1(password: &str, keystore: &KeystoreV1) -> Result<DecryptedIdentity, KeystoreError> {
    if keystore.format != "keystore_v1" {
        return Err(KeystoreError::Invalid("unknown format"));
    }
    if keystore.identity_public_key.len() != 33 {
        return Err(KeystoreError::Invalid("bad identity public key length"));
    }
    if keystore.identity_nonce.len() != 12 {
        return Err(KeystoreError::Invalid("bad nonce length"));
    }

    let master = derive_master_key(password.as_bytes(), &keystore.kdf)?;
    let mut identity_key = derive_aead_key(&master, b"identity");
    let aead = ChaCha20Poly1305::new(Key::from_slice(&identity_key));
    let sk_plain = aead
        .decrypt(
            Nonce::from_slice(&keystore.identity_nonce),
            Payload {
                msg: &keystore.identity_ciphertext,
                aad: &aad(b"identity"),
            },
        )
        .map_err(|_| KeystoreError::Decrypt)?;
    identity_key.zeroize();

    let (keypair, secret) = keypair_from_secret_bytes(&sk_plain)?;
    let pub_bytes = keypair.public().try_into_secp256k1().unwrap().to_bytes();
    if pub_bytes.as_ref() != keystore.identity_public_key.as_slice() {
        return Err(KeystoreError::Invalid("identity public key mismatch"));
    }
    Ok(DecryptedIdentity { keypair, secret })
}

impl KeystoreV1 {
    pub fn to_json_string(&self) -> Result<String, KeystoreError> {
        Ok(serde_json::to_string(self)?)
    }

    pub fn from_json_str(s: &str) -> Result<Self, KeystoreError> {
        Ok(serde_json::from_str(s)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_and_unlock_roundtrip() {
        let (ks, id) = create_keystore_v1("pass123", None).unwrap();
        let id2 = unlock_keystore_v1("pass123", &ks).unwrap();
        assert_eq!(id.public_key_hex(), id2.public_key_hex());
    }

    #[test]
    fn import_secret_matches_create() {
        let (ks1, id1) = create_keystore_v1("pw", None).unwrap();
        let sk = id1.secp256k1_secret().secret_bytes();
        let (ks2, id2) = create_keystore_v1_from_secret("pw2", &sk, None).unwrap();
        assert_eq!(id1.public_key_hex(), id2.public_key_hex());
        assert_ne!(ks1.identity_ciphertext, ks2.identity_ciphertext);
        let id3 = unlock_keystore_v1("pw2", &ks2).unwrap();
        assert_eq!(id1.public_key_hex(), id3.public_key_hex());
    }

    #[test]
    fn parse_secret_key_hex_rejects_bad_length() {
        assert!(parse_secret_key_hex("ab").is_err());
    }

    #[test]
    fn public_key_hex_is_canonical_identity() {
        let (_ks, id) = create_keystore_v1("pw", None).unwrap();
        let pk = id.public_key_hex();
        assert!(crate::public_key_util::same_contact_pk(&pk, &pk));
    }
}
