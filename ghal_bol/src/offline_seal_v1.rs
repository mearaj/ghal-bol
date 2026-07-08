//! Offline encrypt-to-identity (no active transport session).
//!
//! Used by invite/auxiliary FFI only. **secp256k1 recipients** — ephemeral secp256k1 ECDH
//! per message (identity pubkey is the KEM target). Wire prefix `OFFLINE_CIPHER_SECP256K1_V1`.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use rand_core::{OsRng, RngCore};
use secp256k1::{PublicKey, Secp256k1, SecretKey};
use sha2::{Digest, Sha256};

pub const OFFLINE_CIPHER_SECP256K1_V1: u8 = 0x10;

const AES_GCM_NONCE_LEN: usize = 12;

fn random_secret_key() -> Result<SecretKey, String> {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    SecretKey::from_byte_array(bytes).map_err(|e| format!("ephemeral secret: {e}"))
}

fn aes_key_from_shared(shared: &[u8; 32]) -> Key<Aes256Gcm> {
    let digest = Sha256::digest(shared);
    *Key::<Aes256Gcm>::from_slice(&digest)
}

/// Seal `plaintext` to a secp256k1 public key (33-byte compressed).
pub fn seal_to_secp256k1_public(recipient_pk: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, String> {
    let secp = Secp256k1::new();
    let recipient =
        PublicKey::from_slice(recipient_pk).map_err(|e| format!("recipient pubkey: {e}"))?;
    let ephemeral = random_secret_key()?;
    let ephemeral_pub = ephemeral.public_key(&secp);
    let shared = secp256k1::ecdh::SharedSecret::new(&recipient, &ephemeral);
    let mut shared_arr = [0u8; 32];
    shared_arr.copy_from_slice(&shared.secret_bytes());
    let cipher = Aes256Gcm::new(&aes_key_from_shared(&shared_arr));
    let nonce = [0u8; AES_GCM_NONCE_LEN];
    let ephemeral_bytes = ephemeral_pub.serialize();
    let ct = cipher
        .encrypt(Nonce::from_slice(&nonce), plaintext)
        .map_err(|_| "aes-gcm encrypt failed".to_string())?;
    let mut body = Vec::with_capacity(4 + ephemeral_bytes.len() + nonce.len() + ct.len());
    body.extend_from_slice(&(ephemeral_bytes.len() as u32).to_le_bytes());
    body.extend_from_slice(&ephemeral_bytes);
    body.extend_from_slice(&nonce);
    body.extend_from_slice(&ct);
    let mut out = Vec::with_capacity(1 + body.len());
    out.push(OFFLINE_CIPHER_SECP256K1_V1);
    out.extend_from_slice(&body);
    Ok(out)
}

/// Open offline seal (`OFFLINE_CIPHER_SECP256K1_V1` only).
pub fn open_sealed_secp256k1(recipient_sk: &SecretKey, sealed: &[u8]) -> Result<Vec<u8>, String> {
    if sealed.first() != Some(&OFFLINE_CIPHER_SECP256K1_V1) {
        return Err("offline seal: unsupported cipher prefix".to_string());
    }
    let sealed = &sealed[1..];
    if sealed.len() < 4 + 33 + AES_GCM_NONCE_LEN + 16 {
        return Err("sealed blob too short".to_string());
    }
    let pub_len = u32::from_le_bytes([sealed[0], sealed[1], sealed[2], sealed[3]]) as usize;
    if pub_len == 0 || pub_len > 128 || sealed.len() < 4 + pub_len + AES_GCM_NONCE_LEN + 16 {
        return Err("invalid ephemeral pubkey length".to_string());
    }
    let ep_start = 4;
    let ep_end = ep_start + pub_len;
    let nonce_start = ep_end;
    let nonce_end = nonce_start + AES_GCM_NONCE_LEN;
    let ct_start = nonce_end;
    let ephemeral_pub = PublicKey::from_slice(&sealed[ep_start..ep_end])
        .map_err(|e| format!("ephemeral pubkey: {e}"))?;
    let shared = secp256k1::ecdh::SharedSecret::new(&ephemeral_pub, recipient_sk);
    let mut shared_arr = [0u8; 32];
    shared_arr.copy_from_slice(&shared.secret_bytes());
    let cipher = Aes256Gcm::new(&aes_key_from_shared(&shared_arr));
    let nonce = &sealed[nonce_start..nonce_end];
    cipher
        .decrypt(Nonce::from_slice(nonce), &sealed[ct_start..])
        .map_err(|_| "aes-gcm decrypt failed".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use secp256k1::Secp256k1;

    #[test]
    fn roundtrip() {
        let secp = Secp256k1::new();
        let sk = random_secret_key().unwrap();
        let pk = sk.public_key(&secp).serialize();
        let sealed = seal_to_secp256k1_public(&pk, b"hello ghal-bol").unwrap();
        assert_eq!(sealed.first(), Some(&OFFLINE_CIPHER_SECP256K1_V1));
        let plain = open_sealed_secp256k1(&sk, &sealed).unwrap();
        assert_eq!(plain, b"hello ghal-bol");
    }
}
