//! Parse coord endpoints into dial addresses (libp2p dials via `Multiaddr` on the wire).

use std::fmt;
use std::net::SocketAddr;
use std::str::FromStr;

use crate::coord::CoordEndpoint;

/// A TCP (or future QUIC) endpoint peers can dial.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DmDialAddr {
    pub host: String,
    pub port: u16,
}

impl DmDialAddr {
    pub fn new(host: impl Into<String>, port: u16) -> Self {
        Self {
            host: host.into(),
            port,
        }
    }

    pub fn to_socket_addr(&self) -> Result<SocketAddr, String> {
        let s = format!("{}:{}", self.host.trim(), self.port);
        SocketAddr::from_str(&s).map_err(|e| format!("socket addr {s}: {e}"))
    }

    /// libp2p-style string for logs / legacy JSON (`/ip4/…/tcp/…`).
    pub fn to_multiaddr_string(&self) -> String {
        let host = self.host.trim();
        if host.contains(':') {
            format!("/ip6/{host}/tcp/{}", self.port)
        } else {
            format!("/ip4/{host}/tcp/{}", self.port)
        }
    }

    /// Parse `/ip4/h/tcp/p`, `/ip6/h/tcp/p`, or `host:port`.
    pub fn parse(s: &str) -> Option<Self> {
        let t = s.trim();
        if t.is_empty() {
            return None;
        }
        if let Ok(sa) = SocketAddr::from_str(t) {
            return Some(Self::new(sa.ip().to_string(), sa.port()));
        }
        let parts: Vec<&str> = t.split('/').filter(|p| !p.is_empty()).collect();
        if parts.len() >= 4 && (parts[0] == "ip4" || parts[0] == "ip6") && parts[2] == "tcp" {
            if let Ok(port) = parts[3].parse::<u16>() {
                return Some(Self::new(parts[1].to_string(), port));
            }
        }
        None
    }
}

impl fmt::Display for DmDialAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.host, self.port)
    }
}

impl From<CoordEndpoint> for DmDialAddr {
    fn from(ep: CoordEndpoint) -> Self {
        Self::new(ep.host, ep.port)
    }
}

pub fn coord_endpoints_to_dial_addrs(endpoints: &[CoordEndpoint]) -> Vec<DmDialAddr> {
    endpoints
        .iter()
        .filter(|e| e.scheme == "tcp" || e.scheme == "quic")
        .map(|e| DmDialAddr::from(e.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_multiaddr_and_host_port() {
        let a = DmDialAddr::parse("/ip4/127.0.0.1/tcp/8766").unwrap();
        assert_eq!(a.host, "127.0.0.1");
        assert_eq!(a.port, 8766);
        let b = DmDialAddr::parse("192.168.1.2:4433").unwrap();
        assert_eq!(b.port, 4433);
    }
}
