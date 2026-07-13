//! Multi-algorithm **public-key identity** — wire parse/format and per-algorithm codecs.
//!
//! Identity is always a public key. **`algorithm:` prefix optional only for `secp256k1`** (bare hex).
//! All other algorithms require a prefix. **Not** libp2p PeerId.
//! See `docs/MULTI_ALGO.md`.

use serde::{Deserialize, Serialize};

/// Closed enum of supported identity algorithms (wire ids are kebab-case).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum IdentityAlgorithm {
    Secp256k1,
    Ed25519,
    #[serde(rename = "ecdsa-p256")]
    EcdsaP256,
}

impl IdentityAlgorithm {
    pub const fn implicit_default() -> Self {
        Self::Secp256k1
    }

    /// Stable wire / keystore JSON id.
    pub fn wire_id(self) -> &'static str {
        match self {
            Self::Secp256k1 => "secp256k1",
            Self::Ed25519 => "ed25519",
            Self::EcdsaP256 => "ecdsa-p256",
        }
    }

    pub fn from_wire_id(s: &str) -> Result<Self, String> {
        match s.trim() {
            "secp256k1" => Ok(Self::Secp256k1),
            "ed25519" => Ok(Self::Ed25519),
            "ecdsa-p256" => Ok(Self::EcdsaP256),
            other => Err(format!("unknown identity algorithm: {other}")),
        }
    }

    /// Keystore JSON: missing or empty field → implicit `secp256k1` (same rule as wire identity).
    pub fn from_keystore_field(opt: Option<&str>) -> Result<Self, String> {
        match opt.map(str::trim).filter(|s| !s.is_empty()) {
            None => Ok(Self::implicit_default()),
            Some(s) => Self::from_wire_id(s),
        }
    }

    /// Algorithms users may select at first-time identity creation (excludes unimplemented).
    pub fn creatable_algorithms() -> &'static [IdentityAlgorithm] {
        &[
            IdentityAlgorithm::Secp256k1,
            IdentityAlgorithm::Ed25519,
            IdentityAlgorithm::EcdsaP256,
        ]
    }

    /// Whether the shipping libp2p P2P stack can run with this identity.
    pub fn p2p_ready(self) -> bool {
        true
    }

    /// Short UI copy for first-time algorithm selection (product strings live in Rust).
    pub fn create_description(self) -> &'static str {
        match self {
            Self::Secp256k1 => {
                "Default — secp256k1 identity; full chat, calls, and P2P on this build."
            }
            Self::Ed25519 => {
                "Ed25519 identity — full chat, calls, and P2P on this build."
            }
            Self::EcdsaP256 => {
                "NIST P-256 identity — full chat, calls, and P2P on this build."
            }
        }
    }

    /// Hint for raw secret import field (validated in `keystore_v1`).
    pub fn import_secret_hint(self) -> &'static str {
        match self {
            Self::Secp256k1 | Self::Ed25519 => "32-byte secret as 64 hex characters",
            Self::EcdsaP256 => "P-256 secret as even-length hex",
        }
    }
}

/// Parsed identity: algorithm + validated public key bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub algorithm: IdentityAlgorithm,
    pub public_key: Vec<u8>,
}

impl Identity {
    /// Wire string: bare hex (implicit `secp256k1`) or `algorithm:hex`.
    pub fn parse(s: &str) -> Result<Self, String> {
        let t = s.trim();
        if t.is_empty() {
            return Err("identity empty".to_string());
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

    /// Normalize to wire form (lowercase hex; omit prefix for implicit secp256k1).
    pub fn to_wire(&self) -> String {
        let hex = hex::encode(&self.public_key);
        if self.algorithm == IdentityAlgorithm::Secp256k1 {
            hex
        } else {
            format!("{}:{}", self.algorithm.wire_id(), hex)
        }
    }

    /// Bare public key hex when algorithm is implicit `secp256k1` (no prefix on wire).
    pub fn public_key_hex(&self) -> String {
        hex::encode(&self.public_key)
    }

    pub fn from_public_key_bytes(algorithm: IdentityAlgorithm, public_key: Vec<u8>) -> Result<Self, String> {
        validate_public_key(algorithm, &public_key)?;
        Ok(Self {
            algorithm,
            public_key,
        })
    }
}

pub fn validate_public_key(algorithm: IdentityAlgorithm, public_key: &[u8]) -> Result<(), String> {
    match algorithm {
        IdentityAlgorithm::Secp256k1 => {
            secp256k1::PublicKey::from_slice(public_key)
                .map_err(|e| format!("secp256k1 public key: {e}"))?;
            Ok(())
        }
        IdentityAlgorithm::Ed25519 => {
            let arr: [u8; 32] = public_key
                .try_into()
                .map_err(|_| "ed25519 public key: invalid length".to_string())?;
            ed25519_dalek::VerifyingKey::from_bytes(&arr)
                .map_err(|e| format!("ed25519 public key: {e}"))?;
            Ok(())
        }
        IdentityAlgorithm::EcdsaP256 => {
            p256::ecdsa::VerifyingKey::from_sec1_bytes(public_key)
                .map_err(|e| format!("ecdsa-p256 public key: {e}"))?;
            Ok(())
        }
    }
}

pub fn public_key_from_secret(
    algorithm: IdentityAlgorithm,
    secret: &[u8],
) -> Result<Vec<u8>, String> {
    match algorithm {
        IdentityAlgorithm::Secp256k1 => {
            let arr: [u8; 32] = secret
                .try_into()
                .map_err(|_| "secp256k1 secret must be 32 bytes".to_string())?;
            let sk = secp256k1::SecretKey::from_byte_array(arr)
                .map_err(|e| format!("secp256k1 secret: {e}"))?;
            let secp = secp256k1::Secp256k1::new();
            let pk = sk.public_key(&secp);
            let secp = secp256k1::Secp256k1::new();
            Ok(pk.serialize().to_vec())
        }
        IdentityAlgorithm::Ed25519 => {
            let arr: [u8; 32] = secret
                .try_into()
                .map_err(|_| "ed25519 secret must be 32 bytes".to_string())?;
            let signing = ed25519_dalek::SigningKey::from_bytes(&arr);
            Ok(signing.verifying_key().to_bytes().to_vec())
        }
        IdentityAlgorithm::EcdsaP256 => {
            let sk = p256::ecdsa::SigningKey::from_slice(secret)
                .map_err(|e| format!("ecdsa-p256 secret: {e}"))?;
            Ok(sk.verifying_key().to_encoded_point(false).as_bytes().to_vec())
        }
    }
}

pub fn generate_secret(algorithm: IdentityAlgorithm) -> Result<Vec<u8>, String> {
    use rand_core::{OsRng, RngCore};
    match algorithm {
        IdentityAlgorithm::Secp256k1 | IdentityAlgorithm::Ed25519 => {
            let mut b = [0u8; 32];
            OsRng.fill_bytes(&mut b);
            Ok(b.to_vec())
        }
        IdentityAlgorithm::EcdsaP256 => {
            let sk = p256::ecdsa::SigningKey::random(&mut OsRng);
            Ok(sk.to_bytes().to_vec())
        }
    }
}

/// Parse and normalize to canonical wire form (lowercase hex; omit prefix for secp256k1).
pub fn normalize_identity_wire(s: &str) -> Result<String, String> {
    Identity::parse(s).map(|id| id.to_wire())
}

/// Whether two identity wire strings denote the same contact.
pub fn same_contact_identity(a: &str, b: &str) -> bool {
    match (Identity::parse(a), Identity::parse(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

/// Percent-encode a URI path segment (e.g. identity wire with `:`).
pub fn percent_encode_uri_component(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            b' ' => out.push_str("%20"),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Decode percent-encoded URI component.
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

fn decode_pubkey_hex(hex_s: &str) -> Result<Vec<u8>, String> {
    let s = hex_s.trim();
    if s.is_empty() || !s.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("public key: invalid hex".to_string());
    }
    hex::decode(s).map_err(|e| format!("public key hex: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_hex_is_implicit_secp256k1() {
        let (_ks, id) = crate::create_keystore_v1("pw", None).unwrap();
        let wire = id.public_key_hex();
        let parsed = Identity::parse(&wire).unwrap();
        assert_eq!(parsed.algorithm, IdentityAlgorithm::Secp256k1);
    }

    #[test]
    fn unknown_algo_prefix_rejects() {
        assert!(Identity::parse("rsa2048:deadbeef").is_err());
    }

    #[test]
    fn keystore_field_absent_is_secp256k1() {
        assert_eq!(
            IdentityAlgorithm::from_keystore_field(None).unwrap(),
            IdentityAlgorithm::Secp256k1
        );
    }

    #[test]
    fn ed25519_wire_roundtrip() {
        let (_ks, id) =
            crate::create_keystore_v1_with_algorithm("pw", IdentityAlgorithm::Ed25519, None).unwrap();
        let wire = id.identity_wire();
        assert!(wire.starts_with("ed25519:"));
        let parsed = Identity::parse(&wire).unwrap();
        assert_eq!(parsed.to_wire(), wire);
        assert_eq!(normalize_identity_wire(&wire).unwrap(), wire);
    }

    #[test]
    fn ecdsa_p256_wire_roundtrip() {
        let (_ks, id) =
            crate::create_keystore_v1_with_algorithm("pw", IdentityAlgorithm::EcdsaP256, None)
                .unwrap();
        let wire = id.identity_wire();
        assert!(wire.starts_with("ecdsa-p256:"));
        let parsed = Identity::parse(&wire).unwrap();
        assert_eq!(parsed.to_wire(), wire);
    }

    #[test]
    fn ml_dsa65_wire_rejects() {
        assert!(Identity::parse("ml-dsa-65:deadbeef").is_err());
        assert!(IdentityAlgorithm::from_wire_id("ml-dsa-65").is_err());
    }
}
