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

use crate::identity::{self, Identity, IdentityAlgorithm};

const KEYSTORE_V1_AAD_PREFIX: &[u8] = b"ghal_bol.keystore.v1";

#[derive(Debug, Error)]
pub enum KeystoreError {
    #[error("keystore json: {0}")]
    Json(#[from] serde_json::Error),

    #[error("invalid keystore: {0}")]
    Invalid(&'static str),

    #[error("invalid keystore: {0}")]
    InvalidMsg(String),

    #[error("kdf parameters rejected")]
    KdfParams,

    #[error("kdf failed")]
    Kdf,

    #[error("decrypt failed (wrong password or corrupted data)")]
    Decrypt,
}

#[derive(Debug, Error)]
pub enum Libp2pIdentityError {
    #[error("libp2p identity: {0}")]
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
    /// Wire algorithm id (`secp256k1`, `ed25519`, …). **Omitted → implicit `secp256k1`**.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity_algorithm: Option<String>,
    #[serde(with = "serde_bytes")]
    pub identity_public_key: Vec<u8>,
    #[serde(with = "serde_bytes")]
    pub identity_nonce: Vec<u8>,
    #[serde(with = "serde_bytes")]
    pub identity_ciphertext: Vec<u8>,
}

#[derive(Clone)]
enum IdentityKeyMaterial {
    Secp256k1 {
        keypair: Keypair,
        secret: SecretKey,
    },
    Ed25519 {
        signing: ed25519_dalek::SigningKey,
    },
    EcdsaP256 {
        signing: p256::ecdsa::SigningKey,
    },
}

/// Unlocked device identity (multi-algorithm; shipping default is secp256k1).
#[derive(Clone)]
pub struct DecryptedIdentity {
    algorithm: IdentityAlgorithm,
    public_key: Vec<u8>,
    material: IdentityKeyMaterial,
}

impl core::fmt::Debug for DecryptedIdentity {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DecryptedIdentity")
            .field("algorithm", &self.algorithm.wire_id())
            .field("public_key", &"<redacted>")
            .finish()
    }
}

impl DecryptedIdentity {
    pub fn algorithm(&self) -> IdentityAlgorithm {
        self.algorithm
    }

    pub fn keypair(&self) -> &Keypair {
        match &self.material {
            IdentityKeyMaterial::Secp256k1 { keypair, .. } => keypair,
            _ => panic!("keypair() requires secp256k1 identity"),
        }
    }

    pub fn secp256k1_secret(&self) -> &SecretKey {
        match &self.material {
            IdentityKeyMaterial::Secp256k1 { secret, .. } => secret,
            _ => panic!("secp256k1_secret() requires secp256k1 identity"),
        }
    }

    /// Bare public key hex when algorithm is implicit `secp256k1` (no prefix on wire).
    pub fn public_key_hex(&self) -> String {
        hex::encode(&self.public_key)
    }

    /// Full wire identity per `docs/MULTI_ALGO.md`.
    pub fn identity_wire(&self) -> String {
        Identity::from_public_key_bytes(self.algorithm, self.public_key.clone())
            .expect("unlocked identity public key must be valid")
            .to_wire()
    }

    pub fn public_key_bytes(&self) -> &[u8] {
        &self.public_key
    }

    pub fn to_libp2p_keypair(&self) -> Result<Keypair, Libp2pIdentityError> {
        match &self.material {
            IdentityKeyMaterial::Secp256k1 { keypair, .. } => Ok(keypair.clone()),
            IdentityKeyMaterial::Ed25519 { signing } => {
                let mut seed = signing.to_bytes();
                let secret = libp2p_identity::ed25519::SecretKey::try_from_bytes(&mut seed)
                    .map_err(|e| Libp2pIdentityError::SecretKey(format!("{e}")))?;
                Ok(Keypair::from(libp2p_identity::ed25519::Keypair::from(secret)))
            }
            IdentityKeyMaterial::EcdsaP256 { signing } => {
                let sk = libp2p_identity::ecdsa::SecretKey::try_from_bytes(signing.to_bytes())
                    .map_err(|e| Libp2pIdentityError::SecretKey(format!("{e}")))?;
                Ok(Keypair::from(libp2p_identity::ecdsa::Keypair::from(sk)))
            }
        }
    }

    /// Whether this identity can run the shipping libp2p P2P stack.
    pub fn p2p_ready(&self) -> bool {
        self.algorithm.p2p_ready()
    }

    pub(crate) fn ed25519_signing_key(&self) -> Option<&ed25519_dalek::SigningKey> {
        match &self.material {
            IdentityKeyMaterial::Ed25519 { signing } => Some(signing),
            _ => None,
        }
    }

    pub(crate) fn ecdsa_p256_signing_key(&self) -> Option<&p256::ecdsa::SigningKey> {
        match &self.material {
            IdentityKeyMaterial::EcdsaP256 { signing } => Some(signing),
            _ => None,
        }
    }
}

fn derive_master_key(
    password: &[u8],
    kdf: &KeystoreV1KdfParams,
) -> Result<[u8; 32], KeystoreError> {
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

fn aad_legacy_identity() -> Vec<u8> {
    aad(b"identity")
}

fn aad_for_algorithm(algorithm: IdentityAlgorithm) -> Vec<u8> {
    if algorithm == IdentityAlgorithm::Secp256k1 {
        // Legacy on-disk keystores (no identity_algorithm field) use this AAD.
        aad_legacy_identity()
    } else {
        let purpose = format!("identity:{}", algorithm.wire_id());
        aad(purpose.as_bytes())
    }
}

fn aad(purpose: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(KEYSTORE_V1_AAD_PREFIX.len() + 1 + purpose.len());
    v.extend_from_slice(KEYSTORE_V1_AAD_PREFIX);
    v.push(b':');
    v.extend_from_slice(purpose);
    v
}

fn keystore_algorithm(ks: &KeystoreV1) -> Result<IdentityAlgorithm, KeystoreError> {
    IdentityAlgorithm::from_keystore_field(ks.identity_algorithm.as_deref())
        .map_err(|e| KeystoreError::InvalidMsg(e))
}

fn is_legacy_secp256k1_keystore(ks: &KeystoreV1) -> bool {
    ks.identity_algorithm.as_deref().map(str::trim).is_none_or(|s| s.is_empty())
}

fn keypair_from_secp256k1_secret_bytes(sk: &[u8]) -> Result<(Keypair, SecretKey), KeystoreError> {
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

fn decrypted_from_secret(
    algorithm: IdentityAlgorithm,
    secret: &[u8],
) -> Result<DecryptedIdentity, KeystoreError> {
    let public_key = identity::public_key_from_secret(algorithm, secret)
        .map_err(|e| KeystoreError::InvalidMsg(e))?;
    let material = match algorithm {
        IdentityAlgorithm::Secp256k1 => {
            let (keypair, secret) = keypair_from_secp256k1_secret_bytes(secret)?;
            IdentityKeyMaterial::Secp256k1 { keypair, secret }
        }
        IdentityAlgorithm::Ed25519 => {
            let arr: [u8; 32] = secret
                .try_into()
                .map_err(|_| KeystoreError::Invalid("ed25519 secret must be 32 bytes"))?;
            let signing = ed25519_dalek::SigningKey::from_bytes(&arr);
            IdentityKeyMaterial::Ed25519 { signing }
        }
        IdentityAlgorithm::EcdsaP256 => {
            let signing = p256::ecdsa::SigningKey::from_slice(secret)
                .map_err(|_| KeystoreError::Invalid("invalid ecdsa-p256 secret"))?;
            IdentityKeyMaterial::EcdsaP256 { signing }
        }
    };
    Ok(DecryptedIdentity {
        algorithm,
        public_key,
        material,
    })
}

fn encrypt_identity_secret(
    password: &str,
    algorithm: IdentityAlgorithm,
    secret: &[u8],
    public_key: &[u8],
    kdf_override: Option<KeystoreV1KdfParams>,
) -> Result<KeystoreV1, KeystoreError> {
    let mut rng = OsRng;
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
    let mut sk_plain = secret.to_vec();
    let ct = aead
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: &sk_plain,
                aad: &aad_for_algorithm(algorithm),
            },
        )
        .map_err(|_| KeystoreError::Invalid("encrypt identity failed"))?;
    sk_plain.zeroize();
    identity_key.zeroize();

    Ok(KeystoreV1 {
        format: "keystore_v1".to_string(),
        kdf,
        identity_algorithm: if algorithm == IdentityAlgorithm::Secp256k1 {
            None
        } else {
            Some(algorithm.wire_id().to_string())
        },
        identity_public_key: public_key.to_vec(),
        identity_nonce: nonce.to_vec(),
        identity_ciphertext: ct,
    })
}

/// Parse a 32-byte secp256k1 secret as 64 hex digits (legacy import path).
pub fn parse_secret_key_hex(secret_hex: &str) -> Result<[u8; 32], KeystoreError> {
    parse_secret_bytes_for_algorithm(IdentityAlgorithm::Secp256k1, secret_hex)?
        .try_into()
        .map_err(|_| KeystoreError::Invalid("secret key must be 32 bytes"))
}

/// Parse secret key hex for the given identity algorithm.
pub fn parse_secret_bytes_for_algorithm(
    algorithm: IdentityAlgorithm,
    secret_hex: &str,
) -> Result<Vec<u8>, KeystoreError> {
    let s = secret_hex.trim();
    if s.is_empty() || !s.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(KeystoreError::Invalid("invalid secret key hex"));
    }
    if s.len() % 2 != 0 {
        return Err(KeystoreError::Invalid("secret key hex must have even length"));
    }
    let bytes = hex::decode(s).map_err(|_| KeystoreError::Invalid("invalid secret key hex"))?;
    match algorithm {
        IdentityAlgorithm::Secp256k1 | IdentityAlgorithm::Ed25519 => {
            if bytes.len() != 32 {
                return Err(KeystoreError::Invalid(
                    "secret key must be 32 bytes (64 hex chars)",
                ));
            }
        }
        IdentityAlgorithm::EcdsaP256 => {
            if bytes.len() < 32 || bytes.len() > 66 {
                return Err(KeystoreError::Invalid(
                    "ecdsa-p256 secret hex length out of range",
                ));
            }
            p256::ecdsa::SigningKey::from_slice(&bytes)
                .map_err(|_| KeystoreError::Invalid("invalid ecdsa-p256 secret"))?;
        }
    }
    Ok(bytes)
}

/// Create encrypted keystore with the given algorithm (default shipping: secp256k1).
pub fn create_keystore_v1_with_algorithm(
    password: &str,
    algorithm: IdentityAlgorithm,
    kdf_override: Option<KeystoreV1KdfParams>,
) -> Result<(KeystoreV1, DecryptedIdentity), KeystoreError> {
    let secret = identity::generate_secret(algorithm).map_err(|e| KeystoreError::InvalidMsg(e))?;
    create_keystore_v1_from_secret_with_algorithm(password, algorithm, &secret, kdf_override)
}

/// Build an encrypted keystore from an existing secret (import path).
pub fn create_keystore_v1_from_secret_with_algorithm(
    password: &str,
    algorithm: IdentityAlgorithm,
    secret: &[u8],
    kdf_override: Option<KeystoreV1KdfParams>,
) -> Result<(KeystoreV1, DecryptedIdentity), KeystoreError> {
    let id = decrypted_from_secret(algorithm, secret)?;
    let keystore = encrypt_identity_secret(
        password,
        algorithm,
        secret,
        &id.public_key,
        kdf_override,
    )?;
    Ok((keystore, id))
}

/// Build an encrypted keystore from an existing 32-byte secp256k1 secret (legacy import).
pub fn create_keystore_v1_from_secret(
    password: &str,
    secret: &[u8; 32],
    kdf_override: Option<KeystoreV1KdfParams>,
) -> Result<(KeystoreV1, DecryptedIdentity), KeystoreError> {
    create_keystore_v1_from_secret_with_algorithm(
        password,
        IdentityAlgorithm::Secp256k1,
        secret,
        kdf_override,
    )
}

pub fn create_keystore_v1(
    password: &str,
    kdf_override: Option<KeystoreV1KdfParams>,
) -> Result<(KeystoreV1, DecryptedIdentity), KeystoreError> {
    create_keystore_v1_with_algorithm(password, IdentityAlgorithm::Secp256k1, kdf_override)
}

/// Secret key hex for export (secp256k1: 64 hex chars). Other algorithms return raw secret hex.
pub fn secret_key_hex_from_identity(id: &DecryptedIdentity) -> String {
    match &id.material {
        IdentityKeyMaterial::Secp256k1 { secret, .. } => hex::encode(secret.secret_bytes()),
        IdentityKeyMaterial::Ed25519 { signing } => hex::encode(signing.to_bytes()),
        IdentityKeyMaterial::EcdsaP256 { signing } => hex::encode(signing.to_bytes()),
    }
}

pub fn unlock_keystore_v1(
    password: &str,
    keystore: &KeystoreV1,
) -> Result<DecryptedIdentity, KeystoreError> {
    if keystore.format != "keystore_v1" {
        return Err(KeystoreError::Invalid("unknown format"));
    }
    if keystore.identity_nonce.len() != 12 {
        return Err(KeystoreError::Invalid("bad nonce length"));
    }

    let algorithm = keystore_algorithm(keystore)?;

    let master = derive_master_key(password.as_bytes(), &keystore.kdf)?;
    let mut identity_key = derive_aead_key(&master, b"identity");
    let aead = ChaCha20Poly1305::new(Key::from_slice(&identity_key));
    let sk_plain = aead
        .decrypt(
            Nonce::from_slice(&keystore.identity_nonce),
            Payload {
                msg: &keystore.identity_ciphertext,
                aad: &aad_for_algorithm(algorithm),
            },
        )
        .map_err(|_| KeystoreError::Decrypt)?;
    identity_key.zeroize();

    let id = decrypted_from_secret(algorithm, &sk_plain)?;
    if id.public_key.as_slice() != keystore.identity_public_key.as_slice() {
        return Err(KeystoreError::Invalid("identity public key mismatch"));
    }

    // Extra guard for legacy files written before multi-algo (33-byte compressed secp256k1).
    if algorithm == IdentityAlgorithm::Secp256k1 && is_legacy_secp256k1_keystore(keystore) {
        if keystore.identity_public_key.len() != 33 {
            return Err(KeystoreError::Invalid("bad identity public key length"));
        }
    }

    Ok(id)
}

impl KeystoreV1 {
    pub fn to_json_string(&self) -> Result<String, KeystoreError> {
        Ok(serde_json::to_string(self)?)
    }

    pub fn from_json_str(s: &str) -> Result<Self, KeystoreError> {
        Ok(serde_json::from_str(s)?)
    }

    pub fn identity_algorithm_resolved(&self) -> Result<IdentityAlgorithm, KeystoreError> {
        keystore_algorithm(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_and_unlock_roundtrip() {
        let (ks, id) = create_keystore_v1("pass123", None).unwrap();
        assert!(ks.identity_algorithm.is_none());
        let id2 = unlock_keystore_v1("pass123", &ks).unwrap();
        assert_eq!(id.public_key_hex(), id2.public_key_hex());
    }

    #[test]
    fn legacy_json_without_algorithm_field_unlocks() {
        let (ks, id) = create_keystore_v1("pass123", None).unwrap();
        let mut v: serde_json::Value = serde_json::from_str(&ks.to_json_string().unwrap()).unwrap();
        v.as_object_mut().unwrap().remove("identity_algorithm");
        let legacy: KeystoreV1 = serde_json::from_value(v).unwrap();
        let id2 = unlock_keystore_v1("pass123", &legacy).unwrap();
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
    fn ed25519_roundtrip() {
        let (ks, id) =
            create_keystore_v1_with_algorithm("pw", IdentityAlgorithm::Ed25519, None).unwrap();
        assert_eq!(
            ks.identity_algorithm.as_deref(),
            Some("ed25519")
        );
        let id2 = unlock_keystore_v1("pw", &ks).unwrap();
        assert_eq!(id.algorithm(), IdentityAlgorithm::Ed25519);
        assert_eq!(id.public_key_hex(), id2.public_key_hex());
        assert!(id.identity_wire().starts_with("ed25519:"));
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
