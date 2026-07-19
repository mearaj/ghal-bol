//! Contact identity wire + secp256k1 transport helpers.

use secp256k1::PublicKey as Secp256k1PublicKey;

use crate::identity::{normalize_identity_wire, same_contact_identity};

/// Parse compressed secp256k1 public key hex (legacy invite/seal helpers).
pub fn secp256k1_public_key_from_hex(hex_s: &str) -> Result<Secp256k1PublicKey, String> {
    let s = hex_s.trim();
    let v = hex::decode(s).map_err(|e| format!("public_key_hex: hex: {e}"))?;
    Secp256k1PublicKey::from_slice(&v).map_err(|e| format!("public_key_hex: secp256k1: {e}"))
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

fn secp256k1_protobuf_bytes(pk: &[u8]) -> Option<Vec<u8>> {
    if pk.len() != 33 {
        return None;
    }
    let mut out = vec![0x08, 0x02, 0x12, 0x21];
    out.extend_from_slice(pk);
    Some(out)
}

fn multihash_identity_bytes(digest: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(2 + digest.len());
    out.push(0x00);
    out.push(
        digest
            .len()
            .try_into()
            .expect("legacy libp2p protobuf must fit u8"),
    );
    out.extend_from_slice(digest);
    out
}

fn legacy_peer_id_from_secp256k1_bytes(pk: &[u8]) -> Option<String> {
    let proto = secp256k1_protobuf_bytes(pk)?;
    let mh = multihash_identity_bytes(&proto);
    Some(bs58::encode(mh).into_string())
}

fn secp256k1_hex_from_legacy_peer_id(peer_id_str: &str) -> Option<String> {
    let bytes = bs58::decode(peer_id_str.trim()).into_vec().ok()?;
    if bytes.len() < 2 || bytes[0] != 0x00 {
        return None;
    }
    let len = bytes[1] as usize;
    let proto = bytes.get(2..2 + len)?;
    if proto.len() == 37 && proto.starts_with(&[0x08, 0x02, 0x12, 0x21]) {
        return Some(hex::encode(&proto[4..]));
    }
    None
}

/// Load legacy `contacts_v1` rows that only stored `libp2p_peer_id` (pre–pk-only migration).
pub fn legacy_public_key_from_peer_id_str(peer_id_str: &str) -> Option<String> {
    if let Ok(wire) = normalize_contact_identity_wire(peer_id_str) {
        return Some(wire);
    }
    secp256k1_hex_from_legacy_peer_id(peer_id_str).or_else(|| {
        crate::peer_id_util::identity_wire_from_peer_id(peer_id_str)
    })
}

/// Pre–pk-only transcript threads were keyed by libp2p PeerId strings — include when loading.
pub fn legacy_libp2p_peer_id_str_from_public_key_hex(hex_s: &str) -> Option<String> {
    let pk_bytes = hex::decode(hex_s.trim()).ok()?;
    legacy_peer_id_from_secp256k1_bytes(&pk_bytes)
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

    #[test]
    fn legacy_peer_id_roundtrip_secp256k1() {
        let (_ks, id) = create_keystore_v1("t", None).unwrap();
        let pk = id.public_key_hex();
        let pid = legacy_libp2p_peer_id_str_from_public_key_hex(&pk).unwrap();
        assert_eq!(legacy_public_key_from_peer_id_str(&pid).as_deref(), Some(pk.as_str()));
    }
}
