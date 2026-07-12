//! Multi-algorithm identity wire parse/validate (coord API only).
//!
//! Mirrors [`ghal_bol_core::identity`] rules — see `docs/MULTI_ALGO.md`.
//! **No colon → implicit `secp256k1`**. Explicit `secp256k1:` is accepted but normalizes to bare hex.
//! Other algorithms require `algorithm:` prefix.

use crate::error::ServerError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdentityAlgorithm {
    Secp256k1,
    Ed25519,
    EcdsaP256,
}

impl IdentityAlgorithm {
    pub fn from_wire_id(s: &str) -> Result<Self, ServerError> {
        match s.trim() {
            "secp256k1" => Ok(Self::Secp256k1),
            "ed25519" => Ok(Self::Ed25519),
            "ecdsa-p256" => Ok(Self::EcdsaP256),
            other => Err(ServerError::BadRequest(format!(
                "unknown identity algorithm: {other}"
            ))),
        }
    }

    fn wire_id(self) -> &'static str {
        match self {
            Self::Secp256k1 => "secp256k1",
            Self::Ed25519 => "ed25519",
            Self::EcdsaP256 => "ecdsa-p256",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub algorithm: IdentityAlgorithm,
    pub public_key: Vec<u8>,
}

impl Identity {
    pub fn parse(s: &str) -> Result<Self, ServerError> {
        let t = s.trim();
        if t.is_empty() {
            return Err(ServerError::BadRequest("identity empty".into()));
        }
        if let Some((algo, hex_part)) = t.split_once(':') {
            let algorithm = IdentityAlgorithm::from_wire_id(algo)?;
            let public_key = decode_pubkey_hex(hex_part)?;
            validate_public_key(algorithm, &public_key)?;
            return Ok(Self {
                algorithm,
                public_key,
            });
        }
        let public_key = decode_pubkey_hex(t)?;
        let algorithm = IdentityAlgorithm::Secp256k1;
        validate_public_key(algorithm, &public_key)?;
        Ok(Self {
            algorithm,
            public_key,
        })
    }

    pub fn to_wire(&self) -> String {
        let hex = hex::encode(&self.public_key);
        if self.algorithm == IdentityAlgorithm::Secp256k1 {
            hex
        } else {
            format!("{}:{}", self.algorithm.wire_id(), hex)
        }
    }
}

pub fn normalize_identity_wire(s: &str) -> Result<String, ServerError> {
    Identity::parse(s).map(|id| id.to_wire())
}

pub fn percent_decode_uri_component(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) =
                u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16)
            {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn decode_pubkey_hex(hex_s: &str) -> Result<Vec<u8>, ServerError> {
    let s = hex_s.trim();
    if s.is_empty() || !s.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(ServerError::BadRequest("public key: invalid hex".into()));
    }
    hex::decode(s).map_err(|e| ServerError::BadRequest(format!("public key hex: {e}")))
}

fn validate_public_key(algorithm: IdentityAlgorithm, public_key: &[u8]) -> Result<(), ServerError> {
    match algorithm {
        IdentityAlgorithm::Secp256k1 => {
            secp256k1::PublicKey::from_slice(public_key)
                .map_err(|e| ServerError::BadRequest(format!("secp256k1 public key: {e}")))?;
        }
        IdentityAlgorithm::Ed25519 => {
            let arr: [u8; 32] = public_key
                .try_into()
                .map_err(|_| ServerError::BadRequest("ed25519 public key: invalid length".into()))?;
            ed25519_dalek::VerifyingKey::from_bytes(&arr)
                .map_err(|e| ServerError::BadRequest(format!("ed25519 public key: {e}")))?;
        }
        IdentityAlgorithm::EcdsaP256 => {
            p256::ecdsa::VerifyingKey::from_sec1_bytes(public_key).map_err(|e| {
                ServerError::BadRequest(format!("ecdsa-p256 public key: {e}"))
            })?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_hex_is_implicit_secp256k1() {
        let pk = "02".repeat(33);
        let id = Identity::parse(&pk).unwrap();
        assert_eq!(id.algorithm, IdentityAlgorithm::Secp256k1);
        assert_eq!(id.to_wire(), pk);
    }

    #[test]
    fn unknown_algo_prefix_rejects() {
        assert!(Identity::parse("rsa2048:deadbeef").is_err());
    }

    #[test]
    fn ed25519_wire_roundtrip() {
        let wire = "ed25519:9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";
        let id = Identity::parse(wire).unwrap();
        assert_eq!(normalize_identity_wire(wire).unwrap(), wire);
        assert_eq!(id.algorithm, IdentityAlgorithm::Ed25519);
    }

    #[test]
    fn ml_dsa65_wire_rejects() {
        assert!(Identity::parse("ml-dsa-65:deadbeef").is_err());
    }
}
