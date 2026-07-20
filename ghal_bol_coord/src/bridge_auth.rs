//! Signed bridge request verification (`docs/GHAL_BOL_CONNECT_V1.md`).

use crate::error::{ApiResult, ServerError};
use crate::identity::{Identity, normalize_identity_wire};
use ed25519_dalek::{Signature as Ed25519Sig, Verifier, VerifyingKey};
use p256::ecdsa::{Signature as EcdsaSig, VerifyingKey as EcdsaVerifyingKey};
use secp256k1::{Message, PublicKey, Secp256k1, ecdsa::Signature};
use sha2::{Digest, Sha256};

/// Canonical bridge request bytes.
pub fn bridge_request_bytes(
    nonce: &[u8; 32],
    caller_wire: &str,
    peer_identity_wire: &str,
    call_id: &str,
) -> Vec<u8> {
    format!(
        "ghal_bol:bridge:request:v1\n{}\n{}\n{}\n{}",
        hex::encode(nonce),
        caller_wire.trim().to_ascii_lowercase(),
        peer_identity_wire.trim().to_ascii_lowercase(),
        call_id.trim()
    )
    .into_bytes()
}

pub fn verify_bridge_request_signature(
    caller_wire: &str,
    nonce: &[u8; 32],
    peer_identity_wire: &str,
    call_id: &str,
    signature: &[u8],
) -> ApiResult<()> {
    let wire = normalize_identity_wire(caller_wire)?;
    let msg = bridge_request_bytes(nonce, &wire, peer_identity_wire, call_id);
    let id = Identity::parse(&wire)?;
    match id.algorithm {
        crate::identity::IdentityAlgorithm::Secp256k1 => {
            let pk = PublicKey::from_slice(&id.public_key)
                .map_err(|e| ServerError::BadRequest(format!("secp256k1 pk: {e}")))?;
            let hash = Sha256::digest(&msg);
            let digest = Message::from_digest(hash.into());
            let sig = Signature::from_der(signature)
                .map_err(|e| ServerError::Unauthorized(format!("secp256k1 sig: {e}")))?;
            Secp256k1::new()
                .verify_ecdsa(digest, &sig, &pk)
                .map_err(|e| ServerError::Unauthorized(format!("secp256k1 verify: {e}")))?;
        }
        crate::identity::IdentityAlgorithm::Ed25519 => {
            let pk = VerifyingKey::from_bytes(
                id.public_key
                    .as_slice()
                    .try_into()
                    .map_err(|_| ServerError::BadRequest("ed25519 pk length".into()))?,
            )
            .map_err(|e| ServerError::BadRequest(format!("ed25519 pk: {e}")))?;
            let sig = Ed25519Sig::from_bytes(
                signature
                    .try_into()
                    .map_err(|_| ServerError::Unauthorized("ed25519 sig length".into()))?,
            );
            pk.verify(&msg, &sig)
                .map_err(|e| ServerError::Unauthorized(format!("ed25519 verify: {e}")))?;
        }
        crate::identity::IdentityAlgorithm::EcdsaP256 => {
            let pk = EcdsaVerifyingKey::from_sec1_bytes(&id.public_key)
                .map_err(|e| ServerError::BadRequest(format!("ecdsa pk: {e}")))?;
            let sig = EcdsaSig::from_der(signature)
                .map_err(|e| ServerError::Unauthorized(format!("ecdsa sig: {e}")))?;
            pk.verify(&msg, &sig)
                .map_err(|e| ServerError::Unauthorized(format!("ecdsa verify: {e}")))?;
        }
    }
    Ok(())
}
