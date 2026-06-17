//! Parse device `public_key_hex` from libp2p identify `agent_version`.

/// `ghal_bol/<version>;pk=<66-hex-secp256k1>` — binds relay reservations to coord identity.
pub fn parse_pk_from_agent_version(agent: &str) -> Option<String> {
    let agent = agent.trim();
    let pk_part = agent
        .split(';')
        .find(|s| s.trim_start().starts_with("pk="))?;
    let hex = pk_part.trim().strip_prefix("pk=")?;
    let hex = hex.trim();
    if hex.len() != 66 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Some(hex.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pk_suffix() {
        let pk = "02".repeat(33);
        let agent = format!("ghal_bol/0.1.0;pk={pk}");
        assert_eq!(
            parse_pk_from_agent_version(&agent).as_deref(),
            Some(pk.as_str())
        );
    }

    #[test]
    fn rejects_short_hex() {
        assert!(parse_pk_from_agent_version("ghal_bol/0.1.0;pk=abcd").is_none());
    }
}
