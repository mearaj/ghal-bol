//! secp256k1 public key hex — sole wire/contact identity.

use libp2p_identity::{PeerId, PublicKey};

/// Parse 66-hex-char compressed secp256k1 public key.
pub fn secp256k1_public_key_from_hex(hex_s: &str) -> Result<libp2p_identity::secp256k1::PublicKey, String> {
    let s = hex_s.trim();
    if s.len() != 66 {
        return Err("public_key_hex: expected 66 hex chars (compressed secp256k1)".to_string());
    }
    let v = hex::decode(s).map_err(|e| format!("public_key_hex: hex: {e}"))?;
    libp2p_identity::secp256k1::PublicKey::try_from_bytes(&v)
        .map_err(|e| format!("public_key_hex: secp256k1: {e}"))
}

/// Hex-encode libp2p secp256k1 public key bytes (33-byte compressed).
pub fn secp256k1_public_key_to_hex(pk: &libp2p_identity::secp256k1::PublicKey) -> String {
    hex::encode(pk.to_bytes())
}

/// Whether two normalized public key hex strings denote the same contact.
pub fn same_contact_pk(a: &str, b: &str) -> bool {
    let a = a.trim().to_ascii_lowercase();
    let b = b.trim().to_ascii_lowercase();
    a.len() == 66 && a == b
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
    use multihash::Multihash;
    const MULTIHASH_IDENTITY_CODE: u64 = 0;
    let mh: &Multihash<64> = peer.as_ref();
    if mh.code() != MULTIHASH_IDENTITY_CODE {
        return None;
    }
    let pk = PublicKey::try_decode_protobuf(mh.digest()).ok()?;
    let secp = pk.try_into_secp256k1().ok()?;
    Some(secp256k1_public_key_to_hex(&secp))
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
