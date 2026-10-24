//! Multi-algorithm identity wire parse/validate.

use crate::error::DeliveryError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdentityAlgorithm {
    Secp256k1,
    Ed25519,
    EcdsaP256,
}

impl IdentityAlgorithm {
    pub fn from_wire_id(s: &str) -> Result<Self, DeliveryError> {
        match s.trim() {
            "secp256k1" => Ok(Self::Secp256k1),
            "ed25519" => Ok(Self::Ed25519),
            "ecdsa-p256" => Ok(Self::EcdsaP256),
            other => Err(DeliveryError::BadRequest(format!(
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
    pub fn parse(s: &str) -> Result<Self, DeliveryError> {
        let t = s.trim();
        if t.is_empty() {
            return Err(DeliveryError::BadRequest("identity empty".into()));
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

pub fn normalize_identity_wire(s: &str) -> Result<String, DeliveryError> {
    Identity::parse(s).map(|id| id.to_wire())
}

fn decode_pubkey_hex(hex_s: &str) -> Result<Vec<u8>, DeliveryError> {
    let s = hex_s.trim();
    if s.is_empty() || !s.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(DeliveryError::BadRequest("public key: invalid hex".into()));
    }
    hex::decode(s).map_err(|e| DeliveryError::BadRequest(format!("public key hex: {e}")))
}

fn validate_public_key(
    algorithm: IdentityAlgorithm,
    public_key: &[u8],
) -> Result<(), DeliveryError> {
    match algorithm {
        IdentityAlgorithm::Secp256k1 => {
            secp256k1::PublicKey::from_slice(public_key)
                .map_err(|e| DeliveryError::BadRequest(format!("secp256k1 public key: {e}")))?;
        }
        IdentityAlgorithm::Ed25519 => {
            let arr: [u8; 32] = public_key.try_into().map_err(|_| {
                DeliveryError::BadRequest("ed25519 public key: invalid length".into())
            })?;
            ed25519_dalek::VerifyingKey::from_bytes(&arr)
                .map_err(|e| DeliveryError::BadRequest(format!("ed25519 public key: {e}")))?;
        }
        IdentityAlgorithm::EcdsaP256 => {
            p256::ecdsa::VerifyingKey::from_sec1_bytes(public_key).map_err(|e| {
                DeliveryError::BadRequest(format!("ecdsa-p256 public key: {e}"))
            })?;
        }
    }
    Ok(())
}
