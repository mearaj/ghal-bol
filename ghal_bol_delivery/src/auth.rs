//! Challenge nonce issuance and signature verification.

use crate::error::{DeliveryError, Result};
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

#[derive(Clone, Debug)]
pub struct PendingChallenge {
    pub nonce: [u8; 32],
    pub expires_at: Instant,
}

#[derive(Default)]
pub struct ChallengeStore {
    pending: HashMap<String, PendingChallenge>,
    op_nonces: HashMap<String, PendingChallenge>,
}

impl ChallengeStore {
    pub fn issue_session(&mut self, identity_wire: &str, ttl: Duration) -> Result<PendingChallenge> {
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

    pub fn take_session_valid(&mut self, identity_wire: &str, nonce: &[u8; 32]) -> Result<()> {
        let key = normalize_identity_wire(identity_wire)?;
        let Some(ch) = self.pending.remove(&key) else {
            return Err(DeliveryError::Unauthorized(
                "no pending challenge for this identity".into(),
            ));
        };
        if Instant::now() > ch.expires_at {
            return Err(DeliveryError::Unauthorized("challenge expired".into()));
        }
        if ch.nonce != *nonce {
            return Err(DeliveryError::Unauthorized("challenge nonce mismatch".into()));
        }
        Ok(())
    }

    pub fn issue_op_nonce(&mut self, identity_wire: &str, ttl: Duration) -> Result<[u8; 32]> {
        let key = normalize_identity_wire(identity_wire)?;
        let mut nonce = [0u8; 32];
        rand::rng().fill_bytes(&mut nonce);
        self.op_nonces.insert(
            key,
            PendingChallenge {
                nonce,
                expires_at: Instant::now() + ttl,
            },
        );
        Ok(nonce)
    }

    pub fn take_op_valid(&mut self, identity_wire: &str, nonce: &[u8; 32]) -> Result<()> {
        let key = normalize_identity_wire(identity_wire)?;
        let Some(ch) = self.op_nonces.get(&key) else {
            return Err(DeliveryError::Unauthorized("no op nonce".into()));
        };
        if Instant::now() > ch.expires_at {
            self.op_nonces.remove(&key);
            return Err(DeliveryError::Unauthorized("op nonce expired".into()));
        }
        if ch.nonce != *nonce {
            return Err(DeliveryError::Unauthorized("op nonce mismatch".into()));
        }
        self.op_nonces.remove(&key);
        Ok(())
    }

    pub fn purge_expired(&mut self) {
        let now = Instant::now();
        self.pending.retain(|_, ch| now <= ch.expires_at);
        self.op_nonces.retain(|_, ch| now <= ch.expires_at);
    }
}

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

pub fn verify_signature(identity_wire: &str, msg: &[u8], signature: &[u8]) -> Result<()> {
    let wire = normalize_identity_wire(identity_wire)?;
    let id = Identity::parse(&wire)?;
    match id.algorithm {
        IdentityAlgorithm::Secp256k1 => {
            let pk = PublicKey::from_slice(&id.public_key)
                .map_err(|e| DeliveryError::Unauthorized(format!("secp256k1 public key: {e}")))?;
            let hash = Sha256::digest(msg);
            let digest_msg = Message::from_digest(hash.into());
            let sig = Signature::from_der(signature)
                .map_err(|e| DeliveryError::Unauthorized(format!("invalid signature: {e}")))?;
            Secp256k1::verification_only()
                .verify_ecdsa(digest_msg, &sig, &pk)
                .map_err(|e| {
                    DeliveryError::Unauthorized(format!("signature verify failed: {e}"))
                })?;
        }
        IdentityAlgorithm::Ed25519 => {
            let arr: [u8; 32] = id.public_key.as_slice().try_into().map_err(|_| {
                DeliveryError::BadRequest("ed25519 public key: invalid length".into())
            })?;
            let vk = VerifyingKey::from_bytes(&arr)
                .map_err(|e| DeliveryError::BadRequest(format!("ed25519 public key: {e}")))?;
            let sig = Ed25519Sig::from_slice(signature)
                .map_err(|e| DeliveryError::Unauthorized(format!("ed25519 signature: {e}")))?;
            vk.verify(msg, &sig)
                .map_err(|e| DeliveryError::Unauthorized(format!("ed25519 verify: {e}")))?;
        }
        IdentityAlgorithm::EcdsaP256 => {
            let vk = EcdsaVerifyingKey::from_sec1_bytes(&id.public_key)
                .map_err(|e| DeliveryError::BadRequest(format!("ecdsa-p256 public key: {e}")))?;
            let sig = EcdsaSig::from_der(signature)
                .or_else(|_| EcdsaSig::from_slice(signature))
                .map_err(|e| DeliveryError::Unauthorized(format!("ecdsa-p256 signature: {e}")))?;
            EcdsaVerifier::verify(&vk, msg, &sig)
                .map_err(|e| DeliveryError::Unauthorized(format!("ecdsa-p256 verify: {e}")))?;
        }
    }
    Ok(())
}

pub fn parse_nonce_hex(hex_s: &str) -> Result<[u8; 32]> {
    let bytes = hex::decode(hex_s.trim())
        .map_err(|e| DeliveryError::BadRequest(format!("nonce hex: {e}")))?;
    let arr: [u8; 32] = bytes
        .try_into()
        .map_err(|_| DeliveryError::BadRequest("nonce must be 32 bytes".into()))?;
    Ok(arr)
}

pub fn parse_signature_hex(hex_s: &str) -> Result<Vec<u8>> {
    hex::decode(hex_s.trim()).map_err(|e| DeliveryError::BadRequest(format!("signature hex: {e}")))
}
