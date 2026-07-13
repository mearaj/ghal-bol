//! Contact identity wire normalization — no libp2p PeerId.

use crate::identity::{Identity, IdentityAlgorithm};

/// Normalize a contact identity wire string.
pub fn session_peer_from_identity_wire(wire: &str) -> Result<String, String> {
    crate::public_key_util::normalize_contact_identity_wire(wire)
}

/// Whether sender wire matches stream peer or an existing roster row.
pub fn sender_identity_matches_stream_peer(
    identity_wire: &str,
    peer: &str,
    roster_wire: Option<&str>,
) -> bool {
    if contact_identity_wire_matches_peer(identity_wire, peer) {
        return true;
    }
    roster_wire.is_some_and(|stored| {
        crate::identity::same_contact_identity(stored, identity_wire)
    })
}

/// Whether a contact identity wire matches a session peer string.
pub fn contact_identity_wire_matches_peer(identity_wire: &str, peer: &str) -> bool {
    match (
        session_peer_from_identity_wire(identity_wire),
        session_peer_from_identity_wire(peer),
    ) {
        (Ok(a), Ok(b)) => a.eq_ignore_ascii_case(&b),
        _ => false,
    }
}

/// Legacy name — identity wire is the session peer key.
pub fn peer_id_from_identity_wire(wire: &str) -> Result<String, String> {
    session_peer_from_identity_wire(wire)
}

/// Legacy name — identity wire is the session peer key.
pub fn identity_wire_from_peer_id(peer: &str) -> Option<String> {
    session_peer_from_identity_wire(peer).ok()
}

/// Find a contact identity wire matching `peer` from candidates.
pub fn identity_wire_matching_peer_id<'a>(
    peer: &str,
    candidates: impl IntoIterator<Item = &'a str>,
) -> Option<String> {
    let Ok(norm) = session_peer_from_identity_wire(peer) else {
        return None;
    };
    if candidates.into_iter().any(|c| {
        session_peer_from_identity_wire(c)
            .ok()
            .is_some_and(|w| w.eq_ignore_ascii_case(&norm))
    }) {
        Some(norm)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::create_keystore_v1_with_algorithm;

    #[test]
    fn secp256k1_public_hex_matches_session_peer() {
        let (_ks, id) = create_keystore_v1_with_algorithm("pw", IdentityAlgorithm::Secp256k1, None)
            .unwrap();
        let wire = id.identity_wire();
        assert!(contact_identity_wire_matches_peer(&wire, &wire));
        assert_eq!(
            identity_wire_from_peer_id(&wire).as_deref(),
            Some(wire.as_str())
        );
    }

    #[test]
    fn ed25519_identity_peer_roundtrip() {
        let (_ks, id) =
            create_keystore_v1_with_algorithm("pw", IdentityAlgorithm::Ed25519, None).unwrap();
        let wire = id.identity_wire();
        assert!(contact_identity_wire_matches_peer(&wire, &wire));
    }

    #[test]
    fn ecdsa_p256_identity_peer_roundtrip() {
        let (_ks, id) =
            create_keystore_v1_with_algorithm("pw", IdentityAlgorithm::EcdsaP256, None).unwrap();
        let wire = id.identity_wire();
        assert!(contact_identity_wire_matches_peer(&wire, &wire));
        assert_eq!(
            identity_wire_matching_peer_id(&wire, [wire.as_str()]).as_deref(),
            Some(wire.as_str())
        );
    }
}
