//! Minimal multiaddr string type (no libp2p-identity dependency).

use std::fmt;
use std::str::FromStr;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Multiaddr(String);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Protocol {
    Ip4(String),
    Ip6(String),
    Tcp(u16),
    P2p(String),
    P2pCircuit,
    Other(String),
}

impl Multiaddr {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn push(&mut self, proto: Protocol) {
        match proto {
            Protocol::P2pCircuit => {
                if !self.0.contains("/p2p-circuit") {
                    if self.0.ends_with('/') {
                        self.0.pop();
                    }
                    self.0.push_str("/p2p-circuit");
                }
            }
            Protocol::P2p(id) => {
                self.0 = format!("{}/p2p/{id}", self.0.trim_end_matches('/'));
            }
            Protocol::Tcp(port) => {
                self.0 = format!("{}/tcp/{port}", self.0.trim_end_matches('/'));
            }
            Protocol::Ip4(ip) => {
                self.0 = format!("/ip4/{ip}");
            }
            Protocol::Ip6(ip) => {
                self.0 = format!("/ip6/{ip}");
            }
            Protocol::Other(seg) => {
                self.0 = format!("{}/{}", self.0.trim_end_matches('/'), seg);
            }
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = Protocol> + '_ {
        self.protocols().into_iter()
    }

    fn protocols(&self) -> Vec<Protocol> {
        let mut out = Vec::new();
        let parts: Vec<&str> = self.0.split('/').filter(|p| !p.is_empty()).collect();
        let mut i = 0;
        while i < parts.len() {
            match parts[i] {
                "ip4" if i + 1 < parts.len() => {
                    out.push(Protocol::Ip4(parts[i + 1].to_string()));
                    i += 2;
                }
                "ip6" if i + 1 < parts.len() => {
                    out.push(Protocol::Ip6(parts[i + 1].to_string()));
                    i += 2;
                }
                "tcp" if i + 1 < parts.len() => {
                    if let Ok(p) = parts[i + 1].parse::<u16>() {
                        out.push(Protocol::Tcp(p));
                    }
                    i += 2;
                }
                "p2p" if i + 1 < parts.len() => {
                    out.push(Protocol::P2p(parts[i + 1].to_string()));
                    i += 2;
                }
                "p2p-circuit" => {
                    out.push(Protocol::P2pCircuit);
                    i += 1;
                }
                other => {
                    out.push(Protocol::Other(other.to_string()));
                    i += 1;
                }
            }
        }
        out
    }
}

impl FromStr for Multiaddr {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let t = s.trim();
        if t.is_empty() {
            return Err("empty multiaddr".into());
        }
        Ok(Self(t.to_string()))
    }
}

impl fmt::Display for Multiaddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl AsRef<str> for Multiaddr {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_push_circuit() {
        let mut ma: Multiaddr = "/ip4/1.2.3.4/tcp/4001/p2p/abc".parse().unwrap();
        ma.push(Protocol::P2pCircuit);
        assert!(ma.as_str().contains("/p2p-circuit"));
    }
}
