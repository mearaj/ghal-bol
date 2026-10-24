//! TCP dial order for coord HTTPS / bridge WSS.
//!
//! Optional `GHALBOL_COORD_DIAL`, then DNS (AAAA before A). No loopback inventing —
//! a blackholed public IPv4 must not block IPv6 that already works.

use std::net::{SocketAddr, ToSocketAddrs};

/// `(host, port)` from `https://host:port` / `http://host:port`.
pub fn parse_coord_authority(base_url: &str) -> Result<(String, u16), String> {
    let t = base_url.trim().trim_end_matches('/');
    let rest = t
        .strip_prefix("https://")
        .or_else(|| t.strip_prefix("http://"))
        .ok_or_else(|| format!("coord url must be http(s):// (got {t})"))?;
    let authority = rest.split('/').next().unwrap_or("").trim();
    if authority.is_empty() {
        return Err("coord url missing host".into());
    }
    let tls = t.starts_with("https://");
    let (host, port) = if let Some((h, p)) = authority.rsplit_once(':') {
        let host = h.trim().trim_start_matches('[').trim_end_matches(']');
        let port: u16 = p
            .parse()
            .map_err(|_| format!("invalid coord port in {authority}"))?;
        (host.to_string(), port)
    } else {
        (
            authority.to_string(),
            if tls { 443 } else { 80 },
        )
    };
    Ok((host, port))
}

fn push_unique(out: &mut Vec<SocketAddr>, addr: SocketAddr) {
    if !out.contains(&addr) {
        out.push(addr);
    }
}

/// Dial targets to try **before** plain system DNS for `host:port`.
///
/// Order: `GHALBOL_COORD_DIAL` (if set), then DNS addresses with IPv6 first.
pub fn coord_dial_overrides(host: &str, port: u16) -> Vec<SocketAddr> {
    let mut out = Vec::new();
    if let Ok(raw) = std::env::var("GHALBOL_COORD_DIAL") {
        let d = raw.trim();
        if !d.is_empty() {
            let candidate = if d.contains(':') {
                d.to_string()
            } else {
                format!("{d}:{port}")
            };
            if let Ok(iter) = candidate.to_socket_addrs() {
                for addr in iter {
                    push_unique(&mut out, addr);
                }
            }
        }
    }
    if let Ok(iter) = (host, port).to_socket_addrs() {
        let mut dns: Vec<SocketAddr> = iter.collect();
        dns.sort_by_key(|a| u8::from(a.is_ipv4()));
        for addr in dns {
            push_unique(&mut out, addr);
        }
    }
    out
}

/// All TCP dial strings for bridge WSS: overrides first, then `host:port`.
pub fn coord_dial_tcp_targets(host: &str, port: u16) -> Vec<String> {
    let mut out: Vec<String> = coord_dial_overrides(host, port)
        .into_iter()
        .map(|a| a.to_string())
        .collect();
    let named = if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    };
    if !out.iter().any(|c| c == &named) {
        out.push(named);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_https_port() {
        let (h, p) = parse_coord_authority("https://coord1.ghalbol.com:8443").unwrap();
        assert_eq!(h, "coord1.ghalbol.com");
        assert_eq!(p, 8443);
    }

    #[test]
    fn dial_targets_include_named_host() {
        let t = coord_dial_tcp_targets("coord1.ghalbol.com", 8443);
        assert!(t.iter().any(|c| c == "coord1.ghalbol.com:8443"));
    }
}
