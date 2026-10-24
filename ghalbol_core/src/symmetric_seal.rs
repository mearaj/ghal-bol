//! AES-256-GCM seal/open with a pre-derived symmetric key (random 12-byte nonce).

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use rand_core::{OsRng, RngCore};

const AES_GCM_NONCE_LEN: usize = 12;

/// Seal `plaintext` with `key` — wire: `nonce || ciphertext+tag`.
pub fn seal_symmetric(key: &[u8; 32], plaintext: &[u8]) -> Result<Vec<u8>, String> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let mut nonce = [0u8; AES_GCM_NONCE_LEN];
    OsRng.fill_bytes(&mut nonce);
    let ct = cipher
        .encrypt(Nonce::from_slice(&nonce), plaintext)
        .map_err(|_| "aes-gcm encrypt failed".to_string())?;
    let mut out = Vec::with_capacity(nonce.len() + ct.len());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    Ok(out)
}

/// Open a blob produced by [`seal_symmetric`].
pub fn open_symmetric(key: &[u8; 32], sealed: &[u8]) -> Result<Vec<u8>, String> {
    if sealed.len() < AES_GCM_NONCE_LEN + 16 {
        return Err("symmetric sealed blob too short".to_string());
    }
    let (nonce, ct) = sealed.split_at(AES_GCM_NONCE_LEN);
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    cipher
        .decrypt(Nonce::from_slice(nonce), ct)
        .map_err(|_| "aes-gcm decrypt failed".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let key = [7u8; 32];
        let sealed = seal_symmetric(&key, b"dm session").unwrap();
        let plain = open_symmetric(&key, &sealed).unwrap();
        assert_eq!(plain, b"dm session");
    }
}
