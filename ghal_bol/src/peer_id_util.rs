//! Derive libp2p **PeerId** from a secp256k1 public key (hex).

use libp2p::identity::PeerId;
use libp2p::multihash::Multihash;
use libp2p_identity::PublicKey;

const MULTIHASH_IDENTITY_CODE: u64 = 0;

/// Compressed secp256k1 pubkey as hex (66 chars) → libp2p `PeerId` string (`12D3KooW…`).
pub fn peer_id_from_secp256k1_public_key_hex(hex_s: &str) -> Result<String, String> {
    let pk = secp256k1_public_key_from_hex(hex_s)?;
    Ok(PublicKey::from(pk).to_peer_id().to_string())
}

/// Parse 66-hex-char compressed secp256k1 public key.
pub fn secp256k1_public_key_from_hex(
    hex_s: &str,
) -> Result<libp2p_identity::secp256k1::PublicKey, String> {
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

/// Whether [public_key_hex] is the identity behind [peer] (libp2p Noise / DM binding).
pub fn secp256k1_public_hex_matches_peer_id(public_key_hex: &str, peer: &PeerId) -> bool {
    peer_id_from_secp256k1_public_key_hex(public_key_hex)
        .ok()
        .and_then(|s| s.parse::<PeerId>().ok())
        == Some(*peer)
}

/// Public key hex for a libp2p peer when the PeerId embeds the key (secp256k1 identity).
pub fn secp256k1_public_key_hex_from_peer_id(peer: &PeerId) -> Option<String> {
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
    fn public_hex_matches_libp2p_peer_id() {
        let (_ks, id) = create_keystore_v1("pw", None).unwrap();
        let sig = id.public_key_hex();
        let peer = id.to_libp2p_keypair().unwrap().public().to_peer_id();
        assert!(secp256k1_public_hex_matches_peer_id(&sig, &peer));
        assert_eq!(
            secp256k1_public_key_hex_from_peer_id(&peer).as_deref(),
            Some(sig.as_str())
        );
    }
}
