//! Per-call symmetric keys for WebRTC **FrameCryptor** (AES-GCM on encoded frames).
//!
//! Uses the device **secp256k1 identity** (same private/public key as chat sign+seal):
//! ECDH(local secret, peer compressed pubkey) → SHA-256 (same mixing as [`crate::secp256k1_seal`])
//! → HKDF with `call_id` and a binding of **both** 66-hex public keys.

use hkdf::Hkdf;
use secp256k1::{PublicKey, SecretKey};
use sha2::{Digest, Sha256};

use crate::keystore_v1::DecryptedIdentity;

const HKDF_MEDIA_INFO: &[u8] = b"ghal_bol_call_media_v1";
const HKDF_RATCHET_INFO: &[u8] = b"ghal_bol_call_media_ratchet_v1";

/// FrameCryptor AES-256 key + ratchet salt for one call with one contact.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CallMediaKeys {
    pub frame_key: [u8; 32],
    pub ratchet_salt: [u8; 32],
}

fn peer_secp256k1_from_hex(peer_public_key_hex: &str) -> Result<PublicKey, String> {
    let libp2p = crate::public_key_util::secp256k1_public_key_from_hex(peer_public_key_hex.trim())?;
    PublicKey::from_slice(&libp2p.to_bytes()).map_err(|e| format!("peer pubkey: {e}"))
}

/// Sorted `local_hex || peer_hex` (lowercase) so both peers expand identical HKDF `info`.
fn contact_pair_binding(
    local_public_key_hex: &str,
    peer_public_key_hex: &str,
) -> Result<Vec<u8>, String> {
    let mut a = local_public_key_hex.trim().to_ascii_lowercase();
    let mut b = peer_public_key_hex.trim().to_ascii_lowercase();
    if a.len() != 66 {
        return Err("local public_key_hex: expected 66 hex chars".to_string());
    }
    if b.len() != 66 {
        return Err("peer public_key_hex: expected 66 hex chars".to_string());
    }
    if a == b {
        return Err("peer public key must differ from local identity".to_string());
    }
    if a > b {
        std::mem::swap(&mut a, &mut b);
    }
    let mut out = Vec::with_capacity(132);
    out.extend_from_slice(a.as_bytes());
    out.extend_from_slice(b.as_bytes());
    Ok(out)
}

/// SHA-256(ECDH shared), matching DM seal key mixing in [`crate::secp256k1_seal`].
fn ecdh_ikm(local_sk: &SecretKey, peer_public_key_hex: &str) -> Result<[u8; 32], String> {
    let peer_pk = peer_secp256k1_from_hex(peer_public_key_hex)?;
    let shared = secp256k1::ecdh::SharedSecret::new(&peer_pk, local_sk);
    let mut shared_arr = [0u8; 32];
    shared_arr.copy_from_slice(&shared.secret_bytes());
    Ok(Sha256::digest(shared_arr).into())
}

fn hkdf_expand(ikm: &[u8], salt: &[u8], info: &[u8]) -> Result<[u8; 32], String> {
    let hk = Hkdf::<Sha256>::new(Some(salt), ikm);
    let mut out = [0u8; 32];
    hk.expand(info, &mut out)
        .map_err(|_| "hkdf expand failed".to_string())?;
    Ok(out)
}

/// Derive FrameCryptor material from **unlocked identity** + contact public key + `call_id`.
pub fn derive_call_media_keys_from_identity(
    ident: &DecryptedIdentity,
    peer_public_key_hex: &str,
    call_id: &str,
) -> Result<CallMediaKeys, String> {
    let local_hex = ident.public_key_hex();
    derive_call_media_keys(
        ident.secp256k1_secret(),
        &local_hex,
        peer_public_key_hex,
        call_id,
    )
}

/// Derive FrameCryptor material using explicit local secret + both public keys (tests / FFI).
pub fn derive_call_media_keys(
    local_sk: &SecretKey,
    local_public_key_hex: &str,
    peer_public_key_hex: &str,
    call_id: &str,
) -> Result<CallMediaKeys, String> {
    let call_id = call_id.trim();
    if call_id.is_empty() {
        return Err("call_id empty".to_string());
    }
    let pair = contact_pair_binding(local_public_key_hex, peer_public_key_hex)?;
    let ikm = ecdh_ikm(local_sk, peer_public_key_hex)?;
    let salt = call_id.as_bytes();

    let mut media_info = Vec::with_capacity(HKDF_MEDIA_INFO.len() + pair.len());
    media_info.extend_from_slice(HKDF_MEDIA_INFO);
    media_info.extend_from_slice(&pair);

    let mut ratchet_info = Vec::with_capacity(HKDF_RATCHET_INFO.len() + pair.len());
    ratchet_info.extend_from_slice(HKDF_RATCHET_INFO);
    ratchet_info.extend_from_slice(&pair);

    Ok(CallMediaKeys {
        frame_key: hkdf_expand(&ikm, salt, &media_info)?,
        ratchet_salt: hkdf_expand(&ikm, salt, &ratchet_info)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use secp256k1::Secp256k1;

    fn pk_hex(sk: &SecretKey) -> String {
        let secp = Secp256k1::new();
        let bytes = sk.public_key(&secp).serialize();
        let libp2p = libp2p_identity::secp256k1::PublicKey::try_from_bytes(&bytes).expect("pk");
        crate::public_key_util::secp256k1_public_key_to_hex(&libp2p)
    }

    #[test]
    fn both_peers_derive_same_media_keys() {
        let a = SecretKey::from_byte_array([1u8; 32]).unwrap();
        let b = SecretKey::from_byte_array([2u8; 32]).unwrap();
        let a_pub = pk_hex(&a);
        let b_pub = pk_hex(&b);
        let call_id = "call-test-uuid";
        let k_ab = derive_call_media_keys(&a, &a_pub, &b_pub, call_id).unwrap();
        let k_ba = derive_call_media_keys(&b, &b_pub, &a_pub, call_id).unwrap();
        assert_eq!(k_ab, k_ba);
    }

    #[test]
    fn different_call_ids_differ() {
        let a = SecretKey::from_byte_array([3u8; 32]).unwrap();
        let b = SecretKey::from_byte_array([4u8; 32]).unwrap();
        let a_pub = pk_hex(&a);
        let b_pub = pk_hex(&b);
        let k1 = derive_call_media_keys(&a, &a_pub, &b_pub, "call-a").unwrap();
        let k2 = derive_call_media_keys(&a, &a_pub, &b_pub, "call-b").unwrap();
        assert_ne!(k1.frame_key, k2.frame_key);
        assert_ne!(k1.ratchet_salt, k2.ratchet_salt);
    }

    #[test]
    fn rejects_call_with_self() {
        let a = SecretKey::from_byte_array([5u8; 32]).unwrap();
        let a_pub = pk_hex(&a);
        assert!(derive_call_media_keys(&a, &a_pub, &a_pub, "x").is_err());
    }
}
