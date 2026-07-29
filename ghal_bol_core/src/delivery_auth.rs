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

pub fn upload_challenge_bytes(nonce: &[u8; 32], message_id: &str, recipient_wire: &str) -> Vec<u8> {
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

/// Verify delivery-server signatures (DER secp256k1 — matches `ghal_bol_delivery::auth`).
pub fn verify_delivery_signature(
    identity_wire: &str,
    msg: &[u8],
    signature: &[u8],
) -> Result<(), String> {
    use crate::identity::Identity;
    use crate::public_key_util::normalize_contact_identity_wire;
    use ed25519_dalek::{Signature as Ed25519Sig, Verifier, VerifyingKey};
    use p256::ecdsa::{
        Signature as EcdsaSig, VerifyingKey as EcdsaVerifyingKey,
        signature::Verifier as EcdsaVerifier,
    };
    use secp256k1::{Message, PublicKey, Secp256k1, ecdsa::Signature as Secp256k1Sig};

    let wire = normalize_contact_identity_wire(identity_wire)?;
    let id = Identity::parse(&wire)?;
    match id.algorithm {
        IdentityAlgorithm::Secp256k1 => {
            let pk = PublicKey::from_slice(&id.public_key)
                .map_err(|e| format!("secp256k1 public key: {e}"))?;
            let hash = Sha256::digest(msg);
            let digest_msg = Message::from_digest(hash.into());
            let sig = Secp256k1Sig::from_der(signature)
                .map_err(|e| format!("secp256k1 signature: {e}"))?;
            Secp256k1::verification_only()
                .verify_ecdsa(digest_msg, &sig, &pk)
                .map_err(|e| format!("secp256k1 verify: {e}"))?;
        }
        IdentityAlgorithm::Ed25519 => {
            let arr: [u8; 32] = id
                .public_key
                .as_slice()
                .try_into()
                .map_err(|_| "ed25519 public key: invalid length".to_string())?;
            let vk =
                VerifyingKey::from_bytes(&arr).map_err(|e| format!("ed25519 public key: {e}"))?;
            let sig =
                Ed25519Sig::from_slice(signature).map_err(|e| format!("ed25519 signature: {e}"))?;
            vk.verify(msg, &sig)
                .map_err(|e| format!("ed25519 verify: {e}"))?;
        }
        IdentityAlgorithm::EcdsaP256 => {
            let vk = EcdsaVerifyingKey::from_sec1_bytes(&id.public_key)
                .map_err(|e| format!("ecdsa-p256 public key: {e}"))?;
            let sig = EcdsaSig::from_der(signature)
                .or_else(|_| EcdsaSig::from_slice(signature))
                .map_err(|e| format!("ecdsa-p256 signature: {e}"))?;
            EcdsaVerifier::verify(&vk, msg, &sig).map_err(|e| format!("ecdsa-p256 verify: {e}"))?;
        }
    }
    Ok(())
}
