//! Expand libp2p `/dns6/…/p2p-circuit` coord endpoints for TCP-only clients.
//!
//! Android DM dials concrete `/ip4/` or `/dns4/` relay circuits only — not `/dns6/`.

use crate::presence::PeerEndpoint;
use std::net::ToSocketAddrs;

/// Add `/dns4/…` aliases and resolved `/ip4/…` / `/ip6/…` circuits for each DNS circuit hop.
pub fn expand_libp2p_circuit_endpoints(endpoints: Vec<PeerEndpoint>) -> Vec<PeerEndpoint> {
    let mut out = endpoints;
    let mut extra: Vec<PeerEndpoint> = Vec::new();
    let circuit_hosts: Vec<String> = out
        .iter()
        .filter(|ep| ep.scheme == "libp2p" && ep.host.contains("/p2p-circuit"))
        .map(|ep| ep.host.clone())
        .collect();
    for host in circuit_hosts {
        if let Some(dns4) = dns6_to_dns4_circuit(&host) {
            push_unique_circuit(&out, &mut extra, dns4);
        }
        for resolved in resolve_libp2p_circuit_dns_to_ip(&host) {
            push_unique_circuit(&out, &mut extra, resolved);
        }
    }
    out.extend(extra);
    out
}

fn push_unique_circuit(out: &[PeerEndpoint], extra: &mut Vec<PeerEndpoint>, host: String) {
    if out.iter().any(|e| e.scheme == "libp2p" && e.host == host) {
        return;
    }
    if extra.iter().any(|e| e.host == host) {
        return;
    }
    extra.push(PeerEndpoint {
        scheme: "libp2p".into(),
        host,
        port: 0,
    });
}

/// `/dns6/host/…` → `/dns4/host/…` (same suffix) for clients that only dial dns4.
fn dns6_to_dns4_circuit(host: &str) -> Option<String> {
    if !host.contains("/dns6/") {
        return None;
    }
    Some(host.replacen("/dns6/", "/dns4/", 1))
}

/// Resolve the `/dns*` relay hop into `/ip6/…` and `/ip4/…` circuit multiaddrs (IPv6 first).
fn resolve_libp2p_circuit_dns_to_ip(host: &str) -> Vec<String> {
    if host.contains("/ip4/") || host.contains("/ip6/") {
        return Vec::new();
    }
    let segs: Vec<&str> = host.split('/').filter(|s| !s.is_empty()).collect();
    let mut dns_host: Option<&str> = None;
    let mut port: Option<u16> = None;
    let mut p2p_idx: Option<usize> = None;
    let mut i = 0;
    while i < segs.len() {
        match segs[i] {
            "dns4" | "dns6" | "dns" | "dnsaddr" => {
                if i + 1 < segs.len() {
                    dns_host = Some(segs[i + 1]);
                    i += 2;
                    continue;
                }
            }
            "tcp" => {
                if i + 1 < segs.len() {
                    port = segs[i + 1].parse().ok();
                    i += 2;
                    continue;
                }
            }
            "p2p" => {
                p2p_idx = Some(i);
                break;
            }
            _ => {}
        }
        i += 1;
    }
    let (Some(dns), Some(p), Some(p2p_start)) = (dns_host, port, p2p_idx) else {
        return Vec::new();
    };
    let suffix = format!("/{}", segs[p2p_start..].join("/"));
    let Ok(resolved) = format!("{dns}:{p}").to_socket_addrs() else {
        return Vec::new();
    };
    let mut v6 = Vec::new();
    let mut v4 = Vec::new();
    for sa in resolved {
        match sa.ip() {
            std::net::IpAddr::V6(ip) => {
                let a = format!("/ip6/{ip}/tcp/{p}{suffix}");
                if !v6.contains(&a) {
                    v6.push(a);
                }
            }
            std::net::IpAddr::V4(ip) if !ip.is_private() && !ip.is_loopback() => {
                let a = format!("/ip4/{ip}/tcp/{p}{suffix}");
                if !v4.contains(&a) {
                    v4.push(a);
                }
            }
            std::net::IpAddr::V4(_) => {}
        }
    }
    v6.extend(v4);
    v6
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dns6_circuit_gains_dns4_alias() {
        let dns6 = "/dns6/coord.ghalbol.com/tcp/4002/p2p/12D3KooWKUiRKKzspUQShSLWVwxgp1HnSAs3EgDLCQbXn5iGHGhF/p2p-circuit/p2p/16Uiu2HAm5zdGNzac9hYfCNQZTnANbxWytcMty9twy7u942fT7MCk";
        let out = expand_libp2p_circuit_endpoints(vec![PeerEndpoint {
            scheme: "libp2p".into(),
            host: dns6.into(),
            port: 0,
        }]);
        assert!(
            out.iter().any(|e| e.host.contains("/dns4/coord.ghalbol.com/tcp/4002")),
            "expected dns4 alias: {:?}",
            out.iter().map(|e| &e.host).collect::<Vec<_>>()
        );
    }
}
