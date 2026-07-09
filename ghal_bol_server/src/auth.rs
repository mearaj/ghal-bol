//! Challenge nonce issuance and per-algorithm registration signature verification.

use crate::error::{ApiResult, ServerError};
use crate::identity::{Identity, IdentityAlgorithm, normalize_identity_wire};
use ed25519_dalek::{Signature as Ed25519Sig, Verifier, VerifyingKey};
use p256::ecdsa::{
    Signature as EcdsaSig, VerifyingKey as EcdsaVerifyingKey,
    signature::Verifier as EcdsaVerifier,
};
use rand::Rng;
use secp256k1::{Message, PublicKey, Secp256k1, ecdsa::Signature};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Pending registration challenge for one identity.
#[derive(Clone, Debug)]
pub struct PendingChallenge {
    pub nonce: [u8; 32],
    pub expires_at: Instant,
}

/// In-memory challenge table (per process; replace with Redis etc. when scaling).
#[derive(Default)]
pub struct ChallengeStore {
    pending: HashMap<String, PendingChallenge>,
}

impl ChallengeStore {
    pub fn issue(&mut self, identity_wire: &str, ttl: Duration) -> ApiResult<PendingChallenge> {
        let key = normalize_identity_wire(identity_wire)?;
        let mut nonce = [0u8; 32];
        rand::rng().fill_bytes(&mut nonce);
        let challenge = PendingChallenge {
            nonce,
            expires_at: Instant::now() + ttl,
        };
        self.pending.insert(key, challenge.clone());
        Ok(challenge)
    }

    pub fn take_valid(&mut self, identity_wire: &str, nonce: &[u8; 32]) -> ApiResult<()> {
        let key = normalize_identity_wire(identity_wire)?;
        let Some(ch) = self.pending.remove(&key) else {
            return Err(ServerError::Unauthorized(
                "no pending challenge for this identity".into(),
            ));
        };
        if Instant::now() > ch.expires_at {
            return Err(ServerError::Unauthorized("challenge expired".into()));
        }
        if ch.nonce != *nonce {
            return Err(ServerError::Unauthorized("challenge nonce mismatch".into()));
        }
        Ok(())
    }

    pub fn purge_expired(&mut self) {
        let now = Instant::now();
        self.pending.retain(|_, ch| now <= ch.expires_at);
    }
}

/// Canonical registration challenge bytes (all algorithms).
pub fn registration_challenge_bytes(nonce: &[u8; 32], identity_wire: &str) -> Vec<u8> {
    format!(
        "ghal_bol:register:v1\n{}\n{}",
        hex::encode(nonce),
        identity_wire.trim().to_ascii_lowercase()
    )
    .into_bytes()
}

/// secp256k1 digest of [`registration_challenge_bytes`] — legacy helper for coord_client.
pub fn registration_message_digest(nonce: &[u8; 32], identity_wire: &str) -> Message {
    let body = registration_challenge_bytes(nonce, identity_wire);
    let hash = Sha256::digest(&body);
    Message::from_digest(hash.into())
}

pub fn verify_registration_signature(
    identity_wire: &str,
    nonce: &[u8; 32],
    signature: &[u8],
) -> ApiResult<()> {
    let wire = normalize_identity_wire(identity_wire)?;
    let id = Identity::parse(&wire)?;
    let challenge = registration_challenge_bytes(nonce, &wire);
    match id.algorithm {
        IdentityAlgorithm::Secp256k1 => {
            let pk = PublicKey::from_slice(&id.public_key)
                .map_err(|e| ServerError::BadRequest(format!("secp256k1 public key: {e}")))?;
            let msg = registration_message_digest(nonce, &wire);
            let sig = Signature::from_der(signature)
                .map_err(|e| ServerError::Unauthorized(format!("invalid signature: {e}")))?;
            Secp256k1::verification_only()
                .verify_ecdsa(msg, &sig, &pk)
                .map_err(|e| ServerError::Unauthorized(format!("signature verify failed: {e}")))?;
        }
        IdentityAlgorithm::Ed25519 => {
            let arr: [u8; 32] = id
                .public_key
                .as_slice()
                .try_into()
                .map_err(|_| ServerError::BadRequest("ed25519 public key: invalid length".into()))?;
            let vk = VerifyingKey::from_bytes(&arr)
                .map_err(|e| ServerError::BadRequest(format!("ed25519 public key: {e}")))?;
            let sig = Ed25519Sig::from_slice(signature)
                .map_err(|e| ServerError::Unauthorized(format!("ed25519 signature: {e}")))?;
            vk.verify(&challenge, &sig)
                .map_err(|e| ServerError::Unauthorized(format!("ed25519 verify: {e}")))?;
        }
        IdentityAlgorithm::EcdsaP256 => {
            let vk = EcdsaVerifyingKey::from_sec1_bytes(&id.public_key)
                .map_err(|e| ServerError::BadRequest(format!("ecdsa-p256 public key: {e}")))?;
            let sig = EcdsaSig::from_der(signature)
                .or_else(|_| EcdsaSig::from_slice(signature))
                .map_err(|e| ServerError::Unauthorized(format!("ecdsa-p256 signature: {e}")))?;
            EcdsaVerifier::verify(&vk, &challenge, &sig)
                .map_err(|e| ServerError::Unauthorized(format!("ecdsa-p256 verify: {e}")))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use secp256k1::SecretKey;

    #[test]
    fn secp256k1_roundtrip_sign_verify() {
        let secp = Secp256k1::new();
        let sk = SecretKey::from_byte_array([1u8; 32]).expect("test key");
        let pk_hex = hex::encode(sk.public_key(&secp).serialize());
        let nonce = [7u8; 32];
        let msg = registration_message_digest(&nonce, &pk_hex);
        let sig = secp.sign_ecdsa(msg, &sk);
        verify_registration_signature(&pk_hex, &nonce, &sig.serialize_der()).unwrap();
    }

    #[test]
    fn ed25519_roundtrip_sign_verify() {
        let signing = SigningKey::from_bytes(&[2u8; 32]);
        let wire = format!("ed25519:{}", hex::encode(signing.verifying_key().to_bytes()));
        let nonce = [8u8; 32];
        let challenge = registration_challenge_bytes(&nonce, &wire);
        let sig = signing.sign(&challenge);
        verify_registration_signature(&wire, &nonce, &sig.to_bytes()).unwrap();
    }

    #[test]
    fn explicit_secp256k1_prefix_roundtrip() {
        let secp = Secp256k1::new();
        let sk = SecretKey::from_byte_array([4u8; 32]).expect("test key");
        let bare = hex::encode(sk.public_key(&secp).serialize());
        let wire = format!("secp256k1:{bare}");
        let normalized = normalize_identity_wire(&wire).unwrap();
        let nonce = [5u8; 32];
        let msg = registration_message_digest(&nonce, &normalized);
        let sig = secp.sign_ecdsa(msg, &sk);
        verify_registration_signature(&wire, &nonce, &sig.serialize_der()).unwrap();
    }
}
