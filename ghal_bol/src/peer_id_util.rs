//! libp2p **PeerId** ↔ contact identity wire (`[algo:]hex` per MULTI_ALGO.md).

use libp2p::identity::PeerId;
use libp2p::multihash::Multihash;
use libp2p_identity::{ecdsa, ed25519, secp256k1, Keypair, PublicKey};

use crate::identity::{Identity, IdentityAlgorithm};

const MULTIHASH_IDENTITY_CODE: u64 = 0;

#[cfg(test)]
/// libp2p hashes protobuf keys longer than 42 bytes (ecdsa-p256 DER) — not reversible from PeerId alone.
const MULTIHASH_SHA256_CODE: u64 = 0x12;

/// Derive libp2p `PublicKey` from validated identity public key bytes.
pub fn libp2p_public_key_from_identity(
    algorithm: IdentityAlgorithm,
    public_key: &[u8],
) -> Result<PublicKey, String> {
    match algorithm {
        IdentityAlgorithm::Secp256k1 => {
            let secp = secp256k1::PublicKey::try_from_bytes(public_key)
                .map_err(|e| format!("secp256k1 public key: {e}"))?;
            Ok(PublicKey::from(secp))
        }
        IdentityAlgorithm::Ed25519 => {
            let ed = ed25519::PublicKey::try_from_bytes(public_key)
                .map_err(|e| format!("ed25519 public key: {e}"))?;
            Ok(PublicKey::from(ed))
        }
        IdentityAlgorithm::EcdsaP256 => {
            let ec = ecdsa::PublicKey::try_from_bytes(public_key)
                .map_err(|e| format!("ecdsa-p256 public key: {e}"))?;
            Ok(PublicKey::from(ec))
        }
        IdentityAlgorithm::MlDsa65 => Err(
            "ml-dsa-65 product identity uses a separate libp2p transport key".to_string(),
        ),
    }
}

/// Identity wire → libp2p `PeerId` when the algorithm embeds in PeerId (not ml-dsa-65).
pub fn peer_id_from_identity_wire(wire: &str) -> Result<PeerId, String> {
    let id = Identity::parse(wire)?;
    let pk = libp2p_public_key_from_identity(id.algorithm, &id.public_key)?;
    Ok(PeerId::from_public_key(&pk))
}

/// Whether a contact identity wire matches [peer] when PeerId embeds the same libp2p key.
pub fn contact_identity_wire_matches_peer_id(identity_wire: &str, peer: &PeerId) -> bool {
    match peer_id_from_identity_wire(identity_wire) {
        Ok(derived) => derived == *peer,
        Err(_) => false,
    }
}

/// Whether sender wire matches stream peer (embeddable algos) or an existing roster row.
pub fn sender_identity_matches_stream_peer(
    identity_wire: &str,
    peer: &PeerId,
    roster_wire: Option<&str>,
) -> bool {
    if contact_identity_wire_matches_peer_id(identity_wire, peer) {
        return true;
    }
    roster_wire.is_some_and(|stored| {
        crate::identity::same_contact_identity(stored, identity_wire)
    })
}

/// Recover contact identity wire from an **inline** identity multihash PeerId (secp256k1 / ed25519).
///
/// ecdsa-p256 and other large protobuf keys use a SHA-256 PeerId digest — use
/// [`identity_wire_matching_peer_id`] against roster/contacts instead.
pub fn identity_wire_from_peer_id(peer: &PeerId) -> Option<String> {
    let mh: &Multihash<64> = peer.as_ref();
    if mh.code() != MULTIHASH_IDENTITY_CODE {
        return None;
    }
    wire_from_inline_identity_multihash(mh.digest())
}

fn wire_from_inline_identity_multihash(digest: &[u8]) -> Option<String> {
    let pk = PublicKey::try_decode_protobuf(digest).ok()?;
    if let Ok(secp) = pk.clone().try_into_secp256k1() {
        return Some(hex::encode(secp.to_bytes()));
    }
    if let Ok(ed) = pk.clone().try_into_ed25519() {
        return Some(format!("ed25519:{}", hex::encode(ed.to_bytes())));
    }
    if let Ok(ec) = pk.clone().try_into_ecdsa() {
        return Identity::from_public_key_bytes(IdentityAlgorithm::EcdsaP256, ec.to_bytes())
            .ok()
            .map(|id| id.to_wire());
    }
    None
}

/// Find a contact identity wire whose derived libp2p PeerId matches `peer`.
///
/// Required for ecdsa-p256 (SHA-256 PeerId) and ml-dsa transport PeerIds.
pub fn identity_wire_matching_peer_id<'a>(
    peer: &PeerId,
    candidates: impl IntoIterator<Item = &'a str>,
) -> Option<String> {
    if let Some(wire) = identity_wire_from_peer_id(peer) {
        return Some(wire);
    }
    candidates.into_iter().find_map(|raw| {
        let wire = Identity::parse(raw.trim()).ok()?.to_wire();
        peer_id_from_identity_wire(&wire)
            .ok()
            .filter(|derived| derived == peer)
            .map(|_| wire)
    })
}

/// Whether `peer` uses a hashed (non-inline) libp2p PeerId — identity wire cannot be decoded from it.
#[cfg(test)]
pub fn peer_id_uses_hashed_public_key(peer: &PeerId) -> bool {
    peer.as_ref().code() == MULTIHASH_SHA256_CODE
}

/// Deterministic libp2p transport key for ml-dsa-65 product identity (ed25519 Noise).
pub fn ml_dsa_transport_keypair_from_seed(seed: &[u8]) -> Result<Keypair, String> {
    use sha2::Sha256;
    let hk = hkdf::Hkdf::<Sha256>::new(None, seed);
    let mut ed_seed = [0u8; 32];
    hk.expand(b"ghal_bol_ml_dsa_libp2p_transport_v1", &mut ed_seed)
        .map_err(|e| format!("ml-dsa transport hkdf: {e}"))?;
    let secret = ed25519::SecretKey::try_from_bytes(&mut ed_seed)
        .map_err(|e| format!("ml-dsa transport ed25519 secret: {e}"))?;
    Ok(Keypair::from(ed25519::Keypair::from(secret)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::create_keystore_v1_with_algorithm;

    #[test]
    fn secp256k1_public_hex_matches_libp2p_peer_id() {
        let (_ks, id) = create_keystore_v1_with_algorithm("pw", IdentityAlgorithm::Secp256k1, None)
            .unwrap();
        let wire = id.identity_wire();
        let peer = id.to_libp2p_keypair().unwrap().public().to_peer_id();
        assert!(contact_identity_wire_matches_peer_id(&wire, &peer));
        assert_eq!(
            identity_wire_from_peer_id(&peer).as_deref(),
            Some(wire.as_str())
        );
    }

    #[test]
    fn ed25519_identity_peer_id_roundtrip() {
        let (_ks, id) =
            create_keystore_v1_with_algorithm("pw", IdentityAlgorithm::Ed25519, None).unwrap();
        let wire = id.identity_wire();
        let peer = id.to_libp2p_keypair().unwrap().public().to_peer_id();
        assert!(contact_identity_wire_matches_peer_id(&wire, &peer));
        assert_eq!(
            identity_wire_from_peer_id(&peer).as_deref(),
            Some(wire.as_str())
        );
    }

    #[test]
    fn ecdsa_p256_identity_peer_id_roundtrip() {
        let (_ks, id) =
            create_keystore_v1_with_algorithm("pw", IdentityAlgorithm::EcdsaP256, None).unwrap();
        let wire = id.identity_wire();
        let peer = id.to_libp2p_keypair().unwrap().public().to_peer_id();
        assert!(contact_identity_wire_matches_peer_id(&wire, &peer));
        assert!(peer_id_uses_hashed_public_key(&peer));
        assert!(identity_wire_from_peer_id(&peer).is_none());
        assert_eq!(
            identity_wire_matching_peer_id(&peer, [wire.as_str()]).as_deref(),
            Some(wire.as_str())
        );
    }

    #[test]
    fn ml_dsa_transport_keypair_deterministic() {
        let seed = [7u8; 32];
        let a = ml_dsa_transport_keypair_from_seed(&seed).unwrap();
        let b = ml_dsa_transport_keypair_from_seed(&seed).unwrap();
        assert_eq!(
            a.public().to_peer_id().to_string(),
            b.public().to_peer_id().to_string()
        );
    }
}
