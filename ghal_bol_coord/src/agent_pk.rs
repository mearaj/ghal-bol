//! Parse device identity wire from libp2p identify `agent_version`.

use crate::identity::normalize_identity_wire;

/// `ghal_bol/<version>;pk=<identity_wire>` — binds relay reservations to coord identity.
///
/// Returns canonical identity wire (bare secp256k1 hex or `algorithm:hex`). Shipping P2P
/// clients still use secp256k1 libp2p keys; ed25519/ecdsa wires are accepted for coord
/// presence when those identities gain transport support.
pub fn parse_pk_from_agent_version(agent: &str) -> Option<String> {
    let agent = agent.trim();
    let pk_part = agent
        .split(';')
        .find(|s| s.trim_start().starts_with("pk="))?;
    let wire = pk_part.trim().strip_prefix("pk=")?;
    normalize_identity_wire(wire.trim()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;

    fn secp_pk_hex(seed: u8) -> String {
        let sk = secp256k1::SecretKey::from_byte_array([seed; 32]).expect("test key");
        let secp = secp256k1::Secp256k1::new();
        hex::encode(sk.public_key(&secp).serialize())
    }

    #[test]
    fn parses_bare_secp256k1() {
        let pk = secp_pk_hex(1);
        let agent = format!("ghal_bol/0.1.0;pk={pk}");
        assert_eq!(
            parse_pk_from_agent_version(&agent).as_deref(),
            Some(pk.as_str())
        );
    }

    #[test]
    fn parses_explicit_secp256k1_prefix() {
        let pk = secp_pk_hex(2);
        let agent = format!("ghal_bol/0.1.0;pk=secp256k1:{pk}");
        assert_eq!(
            parse_pk_from_agent_version(&agent).as_deref(),
            Some(pk.as_str())
        );
    }

    #[test]
    fn parses_ed25519_prefixed_wire() {
        let signing = SigningKey::from_bytes(&[3u8; 32]);
        let wire = format!("ed25519:{}", hex::encode(signing.verifying_key().to_bytes()));
        let agent = format!("ghal_bol/0.1.0;pk={wire}");
        assert_eq!(
            parse_pk_from_agent_version(&agent).as_deref(),
            Some(wire.as_str())
        );
    }

    #[test]
    fn parses_ecdsa_p256_prefixed_wire() {
        let sk = p256::ecdsa::SigningKey::from_bytes(&[4u8; 32].into()).expect("sk");
        let wire = format!(
            "ecdsa-p256:{}",
            hex::encode(sk.verifying_key().to_encoded_point(false).as_bytes())
        );
        let agent = format!("ghal_bol/0.1.0;pk={wire}");
        assert_eq!(
            parse_pk_from_agent_version(&agent).as_deref(),
            Some(wire.as_str())
        );
    }

    #[test]
    fn rejects_short_hex() {
        assert!(parse_pk_from_agent_version("ghal_bol/0.1.0;pk=abcd").is_none());
    }

    #[test]
    fn rejects_unknown_algo_prefix() {
        let agent = "ghal_bol/0.1.0;pk=rsa2048:deadbeef";
        assert!(parse_pk_from_agent_version(agent).is_none());
    }
}
