//! Per-contact **transport** X25519 KEM for DM confidentiality (decoupled from identity secrets).
//!
//! Identity wires bind HKDF `info`; shared secret comes from X25519 ECDH between ephemeral
//! per-node transport keypairs exchanged via `MsgKind::TransportKemHello`.

use x25519_dalek::{PublicKey, StaticSecret};

use sha2::Digest;

use crate::session_key_common::{hkdf_expand_32, identity_pair_binding};

#[cfg(test)]
use rand_core::{OsRng, RngCore};

pub const TRANSPORT_PUBKEY_LEN: usize = 32;
pub const DM_CIPHER_TRANSPORT_V2: u8 = 0x03;
/// Call signaling ciphertext prefix (transport KEM v2).
pub const CALL_CIPHER_TRANSPORT_V2: u8 = 0x04;

const HKDF_DM_TRANSPORT_INFO: &[u8] = b"ghal_bol_dm_transport_v2";
const HKDF_CALL_SIG_TRANSPORT_INFO: &[u8] = b"ghal_bol_call_sig_transport_v2";
const HKDF_CALL_MEDIA_TRANSPORT_INFO: &[u8] = b"ghal_bol_call_media_transport_v2";
const HKDF_CALL_MEDIA_RATCHET_TRANSPORT_INFO: &[u8] = b"ghal_bol_call_media_ratchet_transport_v2";

/// FrameCryptor AES-256 key + ratchet salt for one call (transport KEM binding).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CallMediaTransportKeys {
    pub frame_key: [u8; 32],
    pub ratchet_salt: [u8; 32],
}

fn transport_ecdh_ikm(local_sk: &StaticSecret, peer_pk: &[u8; TRANSPORT_PUBKEY_LEN]) -> [u8; 32] {
    let peer = PublicKey::from(*peer_pk);
    let shared = local_sk.diffie_hellman(&peer);
    sha2::Sha256::digest(shared.as_bytes()).into()
}

#[cfg(test)]
pub(crate) fn generate_transport_keypair() -> (StaticSecret, [u8; TRANSPORT_PUBKEY_LEN]) {
    let mut seed = [0u8; 32];
    OsRng.fill_bytes(&mut seed);
    let sk = StaticSecret::from(seed);
    let pk = PublicKey::from(&sk);
    (sk, *pk.as_bytes())
}

pub fn transport_public_key_bytes(sk: &StaticSecret) -> [u8; TRANSPORT_PUBKEY_LEN] {
    *PublicKey::from(sk).as_bytes()
}

pub fn parse_transport_pubkey_hex(hex_s: &str) -> Result<[u8; TRANSPORT_PUBKEY_LEN], String> {
    let s = hex_s.trim();
    if s.len() != TRANSPORT_PUBKEY_LEN * 2
        || !s.chars().all(|c| c.is_ascii_hexdigit())
    {
        return Err("transport x25519 public key: expected 64 hex chars".to_string());
    }
    let bytes = hex::decode(s).map_err(|e| format!("transport x25519 hex: {e}"))?;
    bytes
        .try_into()
        .map_err(|_| "transport x25519 public key: invalid length".to_string())
}

/// Derive DM AES-256 key from transport ECDH + sorted identity wire pair binding.
pub fn derive_dm_transport_message_key(
    local_sk: &StaticSecret,
    peer_pk: &[u8; TRANSPORT_PUBKEY_LEN],
    local_identity_wire: &str,
    peer_identity_wire: &str,
) -> Result<[u8; 32], String> {
    let ikm = transport_ecdh_ikm(local_sk, peer_pk);
    let pair = identity_pair_binding(local_identity_wire, peer_identity_wire)?;
    let mut info = Vec::with_capacity(HKDF_DM_TRANSPORT_INFO.len() + pair.len());
    info.extend_from_slice(HKDF_DM_TRANSPORT_INFO);
    info.extend_from_slice(&pair);
    hkdf_expand_32(&ikm, b"conv", &info)
}

/// Derive call signaling AES-256 key from transport ECDH + identity wire pair binding.
pub fn derive_call_sig_transport_message_key(
    local_sk: &StaticSecret,
    peer_pk: &[u8; TRANSPORT_PUBKEY_LEN],
    local_identity_wire: &str,
    peer_identity_wire: &str,
) -> Result<[u8; 32], String> {
    let ikm = transport_ecdh_ikm(local_sk, peer_pk);
    let pair = identity_pair_binding(local_identity_wire, peer_identity_wire)?;
    let mut info = Vec::with_capacity(HKDF_CALL_SIG_TRANSPORT_INFO.len() + pair.len());
    info.extend_from_slice(HKDF_CALL_SIG_TRANSPORT_INFO);
    info.extend_from_slice(&pair);
    hkdf_expand_32(&ikm, b"sig", &info)
}

/// Derive call media FrameCryptor keys from transport ECDH + identity wires + `call_id`.
pub fn derive_call_media_transport_keys(
    local_sk: &StaticSecret,
    peer_pk: &[u8; TRANSPORT_PUBKEY_LEN],
    local_identity_wire: &str,
    peer_identity_wire: &str,
    call_id: &str,
) -> Result<CallMediaTransportKeys, String> {
    let call_id = call_id.trim();
    if call_id.is_empty() {
        return Err("call_id empty".to_string());
    }
    let ikm = transport_ecdh_ikm(local_sk, peer_pk);
    let pair = identity_pair_binding(local_identity_wire, peer_identity_wire)?;
    let salt = call_id.as_bytes();

    let mut media_info = Vec::with_capacity(HKDF_CALL_MEDIA_TRANSPORT_INFO.len() + pair.len());
    media_info.extend_from_slice(HKDF_CALL_MEDIA_TRANSPORT_INFO);
    media_info.extend_from_slice(&pair);

    let mut ratchet_info =
        Vec::with_capacity(HKDF_CALL_MEDIA_RATCHET_TRANSPORT_INFO.len() + pair.len());
    ratchet_info.extend_from_slice(HKDF_CALL_MEDIA_RATCHET_TRANSPORT_INFO);
    ratchet_info.extend_from_slice(&pair);

    Ok(CallMediaTransportKeys {
        frame_key: hkdf_expand_32(&ikm, salt, &media_info)?,
        ratchet_salt: hkdf_expand_32(&ikm, salt, &ratchet_info)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_peers_derive_same_transport_dm_key() {
        let (_ks_a, alice) = crate::create_keystore_v1("pw", None).unwrap();
        let (_ks_b, bob) = crate::create_keystore_v1("pw2", None).unwrap();
        let (sk_a, pk_a) = generate_transport_keypair();
        let (sk_b, pk_b) = generate_transport_keypair();
        let wire_a = alice.identity_wire();
        let wire_b = bob.identity_wire();
        let k_ab = derive_dm_transport_message_key(&sk_a, &pk_b, &wire_a, &wire_b).unwrap();
        let k_ba = derive_dm_transport_message_key(&sk_b, &pk_a, &wire_b, &wire_a).unwrap();
        assert_eq!(k_ab, k_ba);
    }

    #[test]
    fn both_peers_derive_same_call_sig_transport_key() {
        let (_ks_a, alice) = crate::create_keystore_v1("pw", None).unwrap();
        let (_ks_b, bob) = crate::create_keystore_v1("pw2", None).unwrap();
        let (sk_a, pk_a) = generate_transport_keypair();
        let (sk_b, pk_b) = generate_transport_keypair();
        let wire_a = alice.identity_wire();
        let wire_b = bob.identity_wire();
        let k_ab =
            derive_call_sig_transport_message_key(&sk_a, &pk_b, &wire_a, &wire_b).unwrap();
        let k_ba =
            derive_call_sig_transport_message_key(&sk_b, &pk_a, &wire_b, &wire_a).unwrap();
        assert_eq!(k_ab, k_ba);
    }

    #[test]
    fn both_peers_derive_same_call_media_transport_keys() {
        let (_ks_a, alice) = crate::create_keystore_v1("pw", None).unwrap();
        let (_ks_b, bob) = crate::create_keystore_v1("pw2", None).unwrap();
        let (sk_a, pk_a) = generate_transport_keypair();
        let (sk_b, pk_b) = generate_transport_keypair();
        let wire_a = alice.identity_wire();
        let wire_b = bob.identity_wire();
        let call_id = "call-transport-test";
        let k_ab = derive_call_media_transport_keys(&sk_a, &pk_b, &wire_a, &wire_b, call_id)
            .unwrap();
        let k_ba = derive_call_media_transport_keys(&sk_b, &pk_a, &wire_b, &wire_a, call_id)
            .unwrap();
        assert_eq!(k_ab, k_ba);
    }
}
