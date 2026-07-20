//! Contact identity wire normalization — no libp2p PeerId.


/// Normalize a contact identity wire string.
pub fn session_peer_from_identity_wire(wire: &str) -> Result<String, String> {
    crate::public_key_util::normalize_contact_identity_wire(wire)
}



/// Legacy name — identity wire is the session peer key.
pub fn peer_id_from_identity_wire(wire: &str) -> Result<String, String> {
    session_peer_from_identity_wire(wire)
}

/// Legacy name — identity wire is the session peer key.
pub fn identity_wire_from_peer_id(peer: &str) -> Option<String> {
    session_peer_from_identity_wire(peer).ok()
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::{create_keystore_v1_with_algorithm, IdentityAlgorithm};

    #[test]
    fn secp256k1_public_hex_matches_session_peer() {
        let (_ks, id) = create_keystore_v1_with_algorithm("pw", IdentityAlgorithm::Secp256k1, None)
            .unwrap();
        let wire = id.identity_wire();
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
        assert_eq!(session_peer_from_identity_wire(&wire).unwrap(), wire);
    }

    #[test]
    fn ecdsa_p256_identity_peer_roundtrip() {
        let (_ks, id) =
            create_keystore_v1_with_algorithm("pw", IdentityAlgorithm::EcdsaP256, None).unwrap();
        let wire = id.identity_wire();
        assert_eq!(peer_id_from_identity_wire(&wire).unwrap(), wire);
    }
}
