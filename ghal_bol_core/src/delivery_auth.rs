//! Delivery server challenge signing.

use sha2::{Digest, Sha256};

use crate::identity::IdentityAlgorithm;
use crate::keystore_v1::DecryptedIdentity;

pub fn session_challenge_bytes(nonce: &[u8; 32], identity_wire: &str) -> Vec<u8> {
    format!(
        "ghal_bol:delivery:session:v1\n{}\n{}",
        hex::encode(nonce),
        identity_wire.trim().to_ascii_lowercase()
    )
    .into_bytes()
}

pub fn upload_challenge_bytes(
    nonce: &[u8; 32],
    message_id: &str,
    recipient_wire: &str,
) -> Vec<u8> {
    format!(
        "ghal_bol:delivery:upload:v1\n{}\n{}\n{}",
        hex::encode(nonce),
        message_id.trim(),
        recipient_wire.trim().to_ascii_lowercase()
    )
    .into_bytes()
}

pub fn extend_challenge_bytes(nonce: &[u8; 32], message_id: &str) -> Vec<u8> {
    format!(
        "ghal_bol:delivery:extend:v1\n{}\n{}",
        hex::encode(nonce),
        message_id.trim()
    )
    .into_bytes()
}

pub fn sign_delivery_challenge(ident: &DecryptedIdentity, msg: &[u8]) -> Result<Vec<u8>, String> {
    match ident.algorithm() {
        IdentityAlgorithm::Secp256k1 => {
            let hash = Sha256::digest(msg);
            let digest = secp256k1::Message::from_digest(hash.into());
            let sig = secp256k1::Secp256k1::new().sign_ecdsa(digest, ident.secp256k1_secret());
            Ok(sig.serialize_der().to_vec())
        }
        IdentityAlgorithm::Ed25519 | IdentityAlgorithm::EcdsaP256 => ident.sign_message(msg),
    }
}
