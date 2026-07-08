//! Contact identity wire + secp256k1 transport helpers.

use libp2p_identity::{PeerId, PublicKey};

use crate::identity::{normalize_identity_wire, same_contact_identity, Identity};

/// Parse compressed secp256k1 public key hex (legacy invite/seal helpers).
pub fn secp256k1_public_key_from_hex(
    hex_s: &str,
) -> Result<libp2p_identity::secp256k1::PublicKey, String> {
    let s = hex_s.trim();
    let v = hex::decode(s).map_err(|e| format!("public_key_hex: hex: {e}"))?;
    libp2p_identity::secp256k1::PublicKey::try_from_bytes(&v)
        .map_err(|e| format!("public_key_hex: secp256k1: {e}"))
}

/// Hex-encode libp2p secp256k1 public key bytes (33-byte compressed).
#[cfg(test)]
pub fn secp256k1_public_key_to_hex(pk: &libp2p_identity::secp256k1::PublicKey) -> String {
    hex::encode(pk.to_bytes())
}

/// Whether two identity wire strings denote the same contact.
pub fn same_contact_pk(a: &str, b: &str) -> bool {
    same_contact_identity(a, b)
}

/// Parse and normalize contact identity wire (`[algo:]hex`).
pub fn normalize_contact_identity_wire(s: &str) -> Result<String, String> {
    normalize_identity_wire(s)
}

/// True when `s` is a valid contact identity wire string.
pub fn is_valid_contact_identity(s: &str) -> bool {
    Identity::parse(s).is_ok()
}

/// Load legacy `contacts_v1` rows that only stored `libp2p_peer_id` (pre–pk-only migration).
pub fn legacy_public_key_from_peer_id_str(peer_id_str: &str) -> Option<String> {
    let peer: PeerId = peer_id_str.trim().parse().ok()?;
    legacy_public_key_from_peer_id(&peer)
}

/// Pre–pk-only transcript threads were keyed by libp2p PeerId strings — include when loading.
pub fn legacy_libp2p_peer_id_str_from_public_key_hex(hex_s: &str) -> Option<String> {
    let secp = secp256k1_public_key_from_hex(hex_s).ok()?;
    let pk = PublicKey::from(secp);
    Some(PeerId::from_public_key(&pk).to_string())
}

fn legacy_public_key_from_peer_id(peer: &PeerId) -> Option<String> {
    crate::peer_id_util::identity_wire_from_peer_id(peer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::create_keystore_v1;

    #[test]
    fn same_contact_pk_matches_case() {
        let (_ks, id) = create_keystore_v1("t", None).unwrap();
        let pk = id.public_key_hex();
        assert!(same_contact_pk(&pk, &pk.to_uppercase()));
    }
}
