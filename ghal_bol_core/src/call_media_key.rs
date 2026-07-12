//! Per-call symmetric keys for native voice/video AES-GCM frames.
//!
//! Derived from **transport KEM** (X25519 hello on DM stream) + HKDF scoped by `call_id`
//! and sorted identity wire pair.

use x25519_dalek::StaticSecret;

use crate::public_key_util::normalize_contact_identity_wire;
use crate::transport_kem_v1::{CallMediaTransportKeys, derive_call_media_transport_keys};

/// FrameCryptor AES-256 key + ratchet salt for one call with one contact.
pub type CallMediaKeys = CallMediaTransportKeys;

/// Derive FrameCryptor material from transport KEM + identity wires + `call_id`.
pub fn derive_call_media_keys_from_transport(
    local_sk: &StaticSecret,
    peer_pk: &[u8; 32],
    local_identity_wire: &str,
    peer_identity_wire: &str,
    call_id: &str,
) -> Result<CallMediaKeys, String> {
    let local_wire = normalize_contact_identity_wire(local_identity_wire)?;
    let peer_wire = normalize_contact_identity_wire(peer_identity_wire)?;
    derive_call_media_transport_keys(local_sk, peer_pk, &local_wire, &peer_wire, call_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport_kem_v1::generate_transport_keypair;

    #[test]
    fn both_peers_derive_same_media_keys() {
        let (_ks_a, alice) = crate::create_keystore_v1("pw", None).unwrap();
        let (_ks_b, bob) = crate::create_keystore_v1("pw2", None).unwrap();
        let (sk_a, pk_a) = generate_transport_keypair();
        let (sk_b, pk_b) = generate_transport_keypair();
        let wire_a = alice.identity_wire();
        let wire_b = bob.identity_wire();
        let call_id = "call-test-uuid";
        let k_ab =
            derive_call_media_keys_from_transport(&sk_a, &pk_b, &wire_a, &wire_b, call_id).unwrap();
        let k_ba =
            derive_call_media_keys_from_transport(&sk_b, &pk_a, &wire_b, &wire_a, call_id).unwrap();
        assert_eq!(k_ab, k_ba);
    }

    #[test]
    fn different_call_ids_differ() {
        let (_ks_a, alice) = crate::create_keystore_v1("pw", None).unwrap();
        let (_ks_b, bob) = crate::create_keystore_v1("pw2", None).unwrap();
        let (sk_a, pk_a) = generate_transport_keypair();
        let (sk_b, pk_b) = generate_transport_keypair();
        let wire_a = alice.identity_wire();
        let wire_b = bob.identity_wire();
        let k1 =
            derive_call_media_keys_from_transport(&sk_a, &pk_b, &wire_a, &wire_b, "call-a").unwrap();
        let k2 =
            derive_call_media_keys_from_transport(&sk_a, &pk_b, &wire_a, &wire_b, "call-b").unwrap();
        assert_ne!(k1.frame_key, k2.frame_key);
        assert_ne!(k1.ratchet_salt, k2.ratchet_salt);
    }

    #[test]
    fn rejects_call_with_self() {
        let (_ks_a, alice) = crate::create_keystore_v1("pw", None).unwrap();
        let (sk_a, pk_a) = generate_transport_keypair();
        let wire_a = alice.identity_wire();
        assert!(derive_call_media_keys_from_transport(&sk_a, &pk_a, &wire_a, &wire_a, "x").is_err());
    }
}
