//! Coord `POST /v1/register` challenge signing (per identity algorithm).

use sha2::{Digest, Sha256};

use crate::identity::IdentityAlgorithm;
use crate::keystore_v1::DecryptedIdentity;

/// Canonical bytes both peers sign for coord registration.
pub fn registration_challenge_bytes(nonce: &[u8; 32], identity_wire: &str) -> Vec<u8> {
    format!(
        "ghal_bol:register:v1\n{}\n{}",
        hex::encode(nonce),
        identity_wire.trim().to_ascii_lowercase()
    )
    .into_bytes()
}

/// Sign registration challenge for `POST /v1/register`.
pub fn sign_coord_registration(
    ident: &DecryptedIdentity,
    nonce: &[u8; 32],
    identity_wire: &str,
) -> Result<Vec<u8>, String> {
    let bytes = registration_challenge_bytes(nonce, identity_wire);
    match ident.algorithm() {
        IdentityAlgorithm::Secp256k1 => {
            let hash = Sha256::digest(&bytes);
            let msg = secp256k1::Message::from_digest(hash.into());
            let sig = secp256k1::Secp256k1::new().sign_ecdsa(msg, ident.secp256k1_secret());
            Ok(sig.serialize_der().to_vec())
        }
        IdentityAlgorithm::Ed25519 | IdentityAlgorithm::EcdsaP256 => {
            ident.sign_message(&bytes)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::create_keystore_v1_with_algorithm;
    use crate::identity::IdentityAlgorithm;
    use crate::identity_sign::verify_identity_signature;

    #[test]
    fn secp256k1_sign_matches_legacy_digest() {
        let (_ks, id) = crate::create_keystore_v1("pw", None).unwrap();
        let wire = id.identity_wire();
        let nonce = [9u8; 32];
        let sig = sign_coord_registration(&id, &nonce, &wire).unwrap();
        let bytes = registration_challenge_bytes(&nonce, &wire);
        let hash = Sha256::digest(&bytes);
        let msg = secp256k1::Message::from_digest(hash.into());
        let secp = secp256k1::Secp256k1::new();
        let sig2 = secp.sign_ecdsa(msg, id.secp256k1_secret());
        assert_eq!(sig, sig2.serialize_der().to_vec());
    }

    #[test]
    fn ed25519_coord_registration_sign_verify() {
        let (_ks, id) =
            create_keystore_v1_with_algorithm("pw", IdentityAlgorithm::Ed25519, None).unwrap();
        let wire = id.identity_wire();
        let nonce = [3u8; 32];
        let sig = sign_coord_registration(&id, &nonce, &wire).unwrap();
        let bytes = registration_challenge_bytes(&nonce, &wire);
        verify_identity_signature(&wire, &bytes, &sig).unwrap();
    }

}
