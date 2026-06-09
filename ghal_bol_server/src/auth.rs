//! Challenge nonce issuance and secp256k1 signature verification.

use crate::error::{ApiResult, ServerError};
use rand::Rng;
use secp256k1::{ecdsa::Signature, Message, PublicKey, Secp256k1};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::time::{Duration, Instant};

const PUBLIC_KEY_HEX_LEN: usize = 66;

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
    pub fn issue(
        &mut self,
        public_key_hex: &str,
        ttl: Duration,
    ) -> ApiResult<PendingChallenge> {
        validate_public_key_hex(public_key_hex)?;
        let mut nonce = [0u8; 32];
        rand::rng().fill_bytes(&mut nonce);
        let challenge = PendingChallenge {
            nonce,
            expires_at: Instant::now() + ttl,
        };
        self.pending
            .insert(public_key_hex.to_ascii_lowercase(), challenge.clone());
        Ok(challenge)
    }

    pub fn take_valid(&mut self, public_key_hex: &str, nonce: &[u8; 32]) -> ApiResult<()> {
        let key = public_key_hex.to_ascii_lowercase();
        let Some(ch) = self.pending.remove(&key) else {
            return Err(ServerError::Unauthorized(
                "no pending challenge for this public key".into(),
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

/// Canonical signed payload for endpoint registration.
pub fn registration_message_digest(nonce: &[u8; 32], public_key_hex: &str) -> Message {
    let body = format!(
        "ghal_bol:register:v1\n{}\n{}",
        hex::encode(nonce),
        public_key_hex.trim().to_ascii_lowercase()
    );
    let hash = Sha256::digest(body.as_bytes());
    Message::from_digest(hash.into())
}

pub fn verify_registration_signature(
    public_key_hex: &str,
    nonce: &[u8; 32],
    signature_der: &[u8],
) -> ApiResult<()> {
    let pk = parse_public_key_hex(public_key_hex)?;
    let msg = registration_message_digest(nonce, public_key_hex);
    let sig = Signature::from_der(signature_der)
        .map_err(|e| ServerError::Unauthorized(format!("invalid signature: {e}")))?;
    Secp256k1::verification_only()
        .verify_ecdsa(msg, &sig, &pk)
        .map_err(|e| ServerError::Unauthorized(format!("signature verify failed: {e}")))?;
    Ok(())
}

pub fn validate_public_key_hex(hex_s: &str) -> ApiResult<()> {
    parse_public_key_hex(hex_s)?;
    Ok(())
}

fn parse_public_key_hex(hex_s: &str) -> ApiResult<PublicKey> {
    let s = hex_s.trim();
    if s.len() != PUBLIC_KEY_HEX_LEN {
        return Err(ServerError::BadRequest(
            "public_key_hex must be 66 hex chars (compressed secp256k1)".into(),
        ));
    }
    let bytes = hex::decode(s).map_err(|e| ServerError::BadRequest(format!("hex decode: {e}")))?;
    PublicKey::from_slice(&bytes)
        .map_err(|e| ServerError::BadRequest(format!("secp256k1 public key: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use secp256k1::Secp256k1;
    use secp256k1::SecretKey;

    #[test]
    fn roundtrip_sign_verify() {
        let secp = Secp256k1::new();
        let sk = SecretKey::from_byte_array([1u8; 32]).expect("test key");
        let pk_hex = hex::encode(sk.public_key(&secp).serialize());
        let nonce = [7u8; 32];
        let msg = registration_message_digest(&nonce, &pk_hex);
        let sig = secp.sign_ecdsa(msg, &sk);
        verify_registration_signature(&pk_hex, &nonce, &sig.serialize_der()).unwrap();
    }
}
