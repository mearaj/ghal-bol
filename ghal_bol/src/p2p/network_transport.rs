//! Network profile detection, relay/coord dial helpers, and listen-address utilities (no Kademlia).

use std::net::{IpAddr, ToSocketAddrs};
use std::str::FromStr;

use libp2p::multiaddr::Protocol;
use libp2p::{Multiaddr, PeerId};

use super::native_log;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct LocalNetworkProfile {
    pub has_rfc1918_ipv4: bool,
    pub has_public_ipv4: bool,
    pub has_cgnat_ipv4: bool,
    pub has_global_ipv6: bool,
    pub has_wifi_iface: bool,
    pub has_cellular_iface: bool,
    pub has_tether_iface: bool,
    pub has_usb_iface: bool,
    /// Carrier CGNAT address when present (changes on Wi‑Fi ↔ mobile or cell handover).
    pub primary_cgnat_ipv4: Option<std::net::Ipv4Addr>,
    /// RFC1918 address on Wi‑Fi/LAN when present.
    pub primary_rfc1918_ipv4: Option<std::net::Ipv4Addr>,
    /// Direct public IPv4 on an interface (VPS / rare home WAN).
    pub primary_public_ipv4: Option<std::net::Ipv4Addr>,
}

/// Detects Wi‑Fi ↔ mobile, CGNAT/LAN IP changes, and direct public IPv4 churn.
pub(crate) fn network_handover_key(p: &LocalNetworkProfile) -> NetworkHandoverKey {
    NetworkHandoverKey {
        active_lan: p.has_active_lan(),
        mobile_path: p.on_mobile_data_path(),
        mode: p.mode_label(),
        cgnat: p.primary_cgnat_ipv4,
        lan_v4: p.primary_rfc1918_ipv4,
        public_v4: p.primary_public_ipv4,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NetworkHandoverKey {
    pub active_lan: bool,
    pub mobile_path: bool,
    pub mode: &'static str,
    pub cgnat: Option<std::net::Ipv4Addr>,
    pub lan_v4: Option<std::net::Ipv4Addr>,
    pub public_v4: Option<std::net::Ipv4Addr>,
}

/// Sorted WAN-relevant listen addrs (relay circuit + public TCP) for drift detection.
pub(crate) fn wan_coord_listen_fingerprint(addrs: &[Multiaddr]) -> Vec<String> {
    let mut keys: Vec<String> = addrs
        .iter()
        .filter(|ma| {
            is_coord_relay_tcp_circuit_multiaddr(ma) || is_coord_register_tcp_multiaddr(ma)
        })
        .map(|ma| ma.to_string())
        .collect();
    keys.sort();
    keys.dedup();
    keys
}

impl LocalNetworkProfile {
    /// Wi‑Fi, tether, or wired LAN — prefer mDNS/direct TCP; do not treat as cellular-only.
    pub(crate) fn has_active_lan(&self) -> bool {
        if self.has_rfc1918_ipv4
            && (self.has_wifi_iface || self.has_tether_iface || self.has_usb_iface)
        {
            return true;
        }
        // Desktop wired Ethernet (and similar): RFC1918 without a cellular-only path.
        if self.has_rfc1918_ipv4 && !self.on_mobile_data_path() {
            return true;
        }
        false
    }

    /// Skip blind `DialOpts::peer_id` dials; use coord/mDNS explicit multiaddrs instead.
    /// Phones on Wi‑Fi still have a cellular iface — that must not disable LAN routed dials.
    pub(crate) fn avoid_blind_routed_dial(&self) -> bool {
        if self.has_active_lan() {
            return false;
        }
        self.has_cellular_iface || self.has_cgnat_ipv4
    }

    pub(crate) fn mode_label(&self) -> &'static str {
        if self.has_active_lan() {
            return "lan";
        }
        if self.has_cellular_iface || self.has_cgnat_ipv4 {
            return "mobile-data";
        }
        if self.has_tether_iface || self.has_usb_iface {
            return "tethering";
        }
        if self.has_public_ipv4 || self.has_global_ipv6 {
            return "wan";
        }
        if self.has_rfc1918_ipv4 {
            return "lan";
        }
        "unknown"
    }

    pub(crate) fn dial_hint(&self) -> &'static str {
        if self.has_active_lan() {
            return "prioritize mDNS/LAN TCP; coord/relay for WAN";
        }
        if self.has_cellular_iface || self.has_cgnat_ipv4 {
            return "prefer coord+relay, keep LAN fallback";
        }
        if self.has_tether_iface || self.has_usb_iface {
            return "prioritize LAN TCP + mDNS";
        }
        if self.has_rfc1918_ipv4 && !self.has_public_ipv4 {
            return "prioritize mDNS/LAN TCP";
        }
        "use coord + mDNS"
    }

    /// Cellular/CGNAT without an active LAN — needs relay for coord when URL is set.
    pub(crate) fn on_mobile_data_path(&self) -> bool {
        !self.has_active_lan() && (self.has_cellular_iface || self.has_cgnat_ipv4)
    }
}

fn iface_name_is_wifi(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.contains("wlan") || n.contains("wifi") || n.starts_with("wl")
}

fn iface_name_is_cellular(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.contains("rmnet")
        || n.contains("ccmni")
        || n.contains("pdp")
        || n.contains("wwan")
        || n.contains("cell")
        || n.contains("ril")
}

fn iface_name_is_tether(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.contains("tether")
        || n.contains("hotspot")
        || n.contains("ap")
        || n.contains("rndis")
        || n.contains("usb")
}

/// Best-effort local network classification for dial strategy/logging:
/// LAN vs WAN vs mobile-data vs tethering.
pub(crate) fn detect_local_network_profile() -> LocalNetworkProfile {
    let mut p = LocalNetworkProfile::default();
    let Ok(ifs) = if_addrs::get_if_addrs() else {
        return p;
    };
    for iface in ifs {
        let name = iface.name.as_str();
        if iface_name_is_wifi(name) {
            p.has_wifi_iface = true;
        }
        if iface_name_is_cellular(name) {
            p.has_cellular_iface = true;
        }
        if iface_name_is_tether(name) {
            p.has_tether_iface = true;
            if name.to_ascii_lowercase().contains("usb") || name.to_ascii_lowercase().contains("rndis")
            {
                p.has_usb_iface = true;
            }
        }
        match iface.addr {
            if_addrs::IfAddr::V4(v4) => {
                let ip = v4.ip;
                if ip.is_loopback() || is_docker_or_link_local_ipv4(ip) {
                    continue;
                }
                if is_cgnat_ipv4(ip) {
                    p.has_cgnat_ipv4 = true;
                    p.primary_cgnat_ipv4 = Some(ip);
                } else if ip.is_private() {
                    p.has_rfc1918_ipv4 = true;
                    p.primary_rfc1918_ipv4 = Some(ip);
                } else if is_public_bootstrap_ipv4(ip) {
                    p.has_public_ipv4 = true;
                    p.primary_public_ipv4 = Some(ip);
                }
            }
            if_addrs::IfAddr::V6(v6) => {
                let ip = v6.ip;
                if ip.is_loopback()
                    || ip.is_unspecified()
                    || ip.is_unique_local()
                    || ip.is_unicast_link_local()
                {
                    continue;
                }
                p.has_global_ipv6 = true;
            }
        }
    }
    p
}

pub(crate) fn is_cgnat_ipv4(ip: std::net::Ipv4Addr) -> bool {
    let o = ip.octets();
    o[0] == 100 && (64..=127).contains(&o[1])
}

/// Public routable IPv4 only — skip loopback, RFC1918, link-local, CGNAT, and docker0 (172.17.x).
pub(crate) fn is_public_bootstrap_ipv4(ip: std::net::Ipv4Addr) -> bool {
    !ip.is_loopback()
        && !ip.is_private()
        && !ip.is_link_local()
        && !is_docker_or_link_local_ipv4(ip)
        && !is_cgnat_ipv4(ip)
}

/// Resolve a Ghal Bol relay advertised by coord (`GET /v1/relay`) into concrete TCP dial
/// multiaddrs `(PeerId, /ip4/<public-ip>/tcp/<port>/p2p/<id>)`.
pub(crate) fn resolve_relay_bootnodes(peer_str: &str, addrs: &[String]) -> Vec<(PeerId, Multiaddr)> {
    let Ok(peer) = PeerId::from_str(peer_str) else {
        native_log::warn("relay", format!("ghalbol relay bad peer id: {peer_str}"));
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let ip4_only: Vec<&String> = addrs.iter().filter(|a| a.contains("/ip4/")).collect();
    let bases: Vec<&String> = if ip4_only.is_empty() {
        addrs.iter().collect()
    } else {
        ip4_only
    };
    for addr in bases {
        for ma in relay_base_addr_to_dial_multiaddrs(addr, peer) {
            if seen.insert(ma.to_string()) {
                out.push((peer, ma));
            }
        }
    }
    out
}

fn relay_base_addr_to_dial_multiaddrs(addr: &str, peer: PeerId) -> Vec<Multiaddr> {
    let segs: Vec<&str> = addr.trim().split('/').filter(|s| !s.is_empty()).collect();
    let mut host: Option<(String, bool)> = None;
    let mut port: Option<u16> = None;
    let mut i = 0;
    while i + 1 < segs.len() {
        match segs[i] {
            "ip4" => host = Some((segs[i + 1].to_string(), false)),
            "dns4" | "dns" | "dns6" => host = Some((segs[i + 1].to_string(), true)),
            "tcp" => port = segs[i + 1].parse().ok(),
            _ => {}
        }
        i += 1;
    }
    let (Some((h, is_dns)), Some(p)) = (host, port) else {
        native_log::warn("relay", format!("ghalbol relay addr not TCP host/port: {addr}"));
        return Vec::new();
    };
    let mut out = Vec::new();
    if is_dns {
        let Ok(resolved) = format!("{h}:{p}").to_socket_addrs() else {
            native_log::warn("relay", format!("ghalbol relay DNS resolve failed: {h}"));
            return Vec::new();
        };
        for sa in resolved {
            if let IpAddr::V4(ip) = sa.ip() {
                if !is_public_bootstrap_ipv4(ip) {
                    continue;
                }
                if let Ok(ma) = format!("/ip4/{ip}/tcp/{p}/p2p/{peer}").parse::<Multiaddr>() {
                    out.push(ma);
                }
            }
        }
    } else if let Ok(ip) = h.parse::<std::net::Ipv4Addr>() {
        if is_public_bootstrap_ipv4(ip) {
            if let Ok(ma) = format!("/ip4/{ip}/tcp/{p}/p2p/{peer}").parse::<Multiaddr>() {
                out.push(ma);
            }
        }
    }
    out
}

pub(crate) fn ipv4_from_ma_str(s: &str) -> Option<std::net::Ipv4Addr> {
    let host = s.split("/ip4/").nth(1)?.split('/').next()?;
    host.parse().ok()
}

/// True when this address is worth publishing to the DHT (WAN + LAN; not loopback/docker/link-local).
pub(crate) fn is_publishable_listen_addr(ma: &Multiaddr) -> bool {
    let s = ma.to_string();
    if s.contains("/ip4/0.0.0.0/") || s.contains("/ip4/127.0.0.1/") || s.contains("/ip6/::1/") {
        return false;
    }
    if let Some(ip) = ipv4_from_ma_str(&s) {
        if ip.is_loopback() || is_docker_or_link_local_ipv4(ip) {
            return false;
        }
    }
    if s.contains("/ip6/fe80:") {
        return false;
    }
    if s.contains("/p2p-circuit") {
        return false;
    }
    s.contains("/tcp/") || s.contains("/quic")
}

/// libp2p relay circuit listen/dial address (NAT traversal for phones).
pub(crate) fn is_relay_circuit_multiaddr(ma: &Multiaddr) -> bool {
    ma.to_string().contains("/p2p-circuit")
}

/// Relay circuit reachable via DM TCP transport (not QUIC/WebRTC/WSS relay hops).
pub(crate) fn is_coord_relay_tcp_circuit_multiaddr(ma: &Multiaddr) -> bool {
    if !is_relay_circuit_multiaddr(ma) {
        return false;
    }
    let s = ma.to_string();
    s.contains("/tcp/") && !s.contains("/quic") && !s.contains("/webrtc") && !s.contains("/wss")
}

/// Addresses we will put in the peerstore / dial for DM (TCP only — matches Android transport).
pub(crate) fn is_dm_dial_multiaddr(ma: &Multiaddr) -> bool {
    if is_relay_circuit_multiaddr(ma) {
        return true;
    }
    let s = ma.to_string();
    if !s.contains("/tcp/") {
        return false;
    }
    if s.contains("/quic") || s.contains("/webrtc") || s.contains("/wss") {
        return false;
    }
    if let Some(ip) = ipv4_from_ma_str(&s) {
        if ip.is_loopback() || is_docker_or_link_local_ipv4(ip) {
            return false;
        }
        // Plain TCP to libp2p bootstrap port 4001 is a relay hop, not the DM peer.
        if let Some(port) = s
            .split("/tcp/")
            .nth(1)
            .and_then(|p| p.split('/').next())
            .and_then(|p| p.parse::<u16>().ok())
        {
            if port == 4001 {
                return false;
            }
        }
        return true;
    }
    false
}

/// TCP multiaddrs for DHT publish + DM dial (LAN + relay `/tcp/.../p2p-circuit`, no QUIC/WebRTC/WSS).
pub(crate) fn is_dm_listen_tcp_multiaddr(ma: &Multiaddr) -> bool {
    if is_relay_circuit_multiaddr(ma) {
        return true;
    }
    let s = ma.to_string();
    if !s.contains("/tcp/") {
        return false;
    }
    if s.contains("/quic") || s.contains("/webrtc") || s.contains("/wss") {
        return false;
    }
    is_publishable_listen_addr(ma)
}

pub(crate) fn tcp_dm_publish_addrs(addrs: Vec<Multiaddr>) -> Vec<Multiaddr> {
    addrs
        .into_iter()
        .filter(|ma| is_dm_listen_tcp_multiaddr(ma))
        .collect()
}

/// Inbound coord lookup addrs to dial when coord is primary (WAN paths only).
pub(crate) fn wan_coord_dial_addrs(addrs: Vec<Multiaddr>) -> Vec<Multiaddr> {
    let filtered: Vec<Multiaddr> = addrs
        .into_iter()
        .filter(|ma| {
            is_coord_relay_tcp_circuit_multiaddr(ma)
                || is_coord_register_tcp_multiaddr(ma)
                || (is_relay_circuit_multiaddr(ma) && is_dm_dial_multiaddr(ma))
        })
        .collect();
    filter_coord_relay_dial_platform(sort_coord_dial_multiaddrs(filtered))
}

/// Sort order for DM dials: LAN TCP first, then relay, then public WAN.
pub(crate) fn dm_dial_addr_rank(ma: &Multiaddr) -> u8 {
    if let Some(ip) = ipv4_from_ma_str(&ma.to_string()) {
        if ip.is_loopback() || is_docker_or_link_local_ipv4(ip) {
            return 4;
        }
        if ip.is_private() {
            return 0;
        }
        return 2;
    }
    if is_relay_circuit_multiaddr(ma) {
        return 1;
    }
    3
}

pub(crate) fn sort_dm_dial_addrs(mut addrs: Vec<Multiaddr>) -> Vec<Multiaddr> {
    addrs.sort_by_key(|ma| dm_dial_addr_rank(ma));
    addrs
}

pub(crate) fn dm_dial_addr_rank_wan_first(ma: &Multiaddr) -> u8 {
    if is_relay_circuit_multiaddr(ma) {
        return 0;
    }
    if let Some(ip) = ipv4_from_ma_str(&ma.to_string()) {
        if ip.is_loopback() || is_docker_or_link_local_ipv4(ip) {
            return 4;
        }
        if ip.is_private() {
            return 3;
        }
        return 1;
    }
    2
}

pub(crate) fn sort_dm_dial_addrs_wan_first(mut addrs: Vec<Multiaddr>) -> Vec<Multiaddr> {
    addrs.sort_by_key(|ma| dm_dial_addr_rank_wan_first(ma));
    addrs
}

/// LAN-first when [peer_on_local_lan] (mDNS); WAN-first (relay, then public) otherwise.
pub(crate) fn rank_dm_dial_addrs_for_peer(
    addrs: Vec<Multiaddr>,
    peer_on_local_lan: bool,
) -> Vec<Multiaddr> {
    if peer_on_local_lan {
        sort_dm_dial_addrs(addrs)
    } else {
        sort_dm_dial_addrs_wan_first(addrs)
    }
}

/// When a routable public IP is present, skip RFC1918 LAN (coord often lists both).
/// LAN + relay only: keep both — try direct TCP before relay circuit.
#[cfg(test)]
pub(crate) fn filter_wan_preferred_dm_dial_addrs(addrs: Vec<Multiaddr>) -> Vec<Multiaddr> {
    let sorted = sort_dm_dial_addrs(addrs);
    let has_routable_public = sorted.iter().any(|ma| {
        if is_relay_circuit_multiaddr(ma) {
            return false;
        }
        ipv4_from_ma_str(&ma.to_string())
            .is_some_and(|ip| !ip.is_private() && !ip.is_loopback())
    });
    if has_routable_public {
        return sorted
            .into_iter()
            .filter(|ma| {
                ipv4_from_ma_str(&ma.to_string())
                    .map(|ip| !ip.is_private())
                    .unwrap_or(true)
            })
            .collect();
    }
    sorted
}

/// Coord registration: public routable TCP only (not RFC1918 — mDNS covers LAN per DESIGN.md).
pub(crate) fn is_coord_register_tcp_multiaddr(ma: &Multiaddr) -> bool {
    if !is_dm_listen_tcp_multiaddr(ma) || is_relay_circuit_multiaddr(ma) {
        return false;
    }
    ipv4_from_ma_str(&ma.to_string())
        .map(is_public_bootstrap_ipv4)
        .unwrap_or(false)
}

/// WAN coord dial filter: public TCP + relay only (drops RFC1918/CGNAT).
pub(crate) fn filter_coord_dial_addrs(addrs: Vec<Multiaddr>) -> Vec<Multiaddr> {
    sort_coord_dial_multiaddrs(
        addrs
            .into_iter()
            .filter(|ma| {
                is_relay_circuit_multiaddr(ma)
                    || ipv4_from_ma_str(&ma.to_string())
                        .map(is_public_bootstrap_ipv4)
                        .unwrap_or(false)
            })
            .collect(),
    )
}

/// Prefer IPv4 TCP relay circuits (Android DM transport is TCP-only).
pub(crate) fn sort_coord_dial_multiaddrs(mut addrs: Vec<Multiaddr>) -> Vec<Multiaddr> {
    addrs.sort_by_key(|ma| {
        let s = ma.to_string();
        if is_relay_circuit_multiaddr(ma) {
            if s.contains("/ip4/") && s.contains("/tcp/") {
                0u8
            } else if s.contains("/ip6/") && s.contains("/tcp/") {
                1
            } else {
                2
            }
        } else {
            3
        }
    });
    addrs
}

/// WAN coord dials: IPv4 TCP relay circuits only (DM transport is TCP; IPv6 relay often fails).
pub(crate) fn filter_coord_relay_dial_platform(mut addrs: Vec<Multiaddr>) -> Vec<Multiaddr> {
    addrs.retain(|ma| !is_relay_circuit_multiaddr(ma) || is_coord_relay_tcp_circuit_multiaddr(ma));
    #[cfg(target_os = "android")]
    {
        addrs.retain(|ma| {
            !is_relay_circuit_multiaddr(ma) || ma.to_string().contains("/ip4/")
        });
    }
    addrs
}

fn local_ipv4_addrs() -> Vec<std::net::Ipv4Addr> {
    let mut out = Vec::new();
    let Ok(ifs) = if_addrs::get_if_addrs() else {
        return out;
    };
    for iface in ifs {
        if let if_addrs::IfAddr::V4(v4) = iface.addr {
            if v4.ip.is_loopback() || is_docker_or_link_local_ipv4(v4.ip) {
                continue;
            }
            let o = v4.ip.octets();
            // RFC1918 LAN addresses (phones and laptops often share 192.168.x or 10.x).
            if o[0] == 10
                || (o[0] == 172 && (16..=31).contains(&o[1]))
                || (o[0] == 192 && o[1] == 168)
            {
                out.push(v4.ip);
            }
        }
    }
    out
}

/// docker0 / podman / common CNI bridges — not reachable for DM dial (see user logs: 172.17–172.20).
fn is_docker_or_link_local_ipv4(ip: std::net::Ipv4Addr) -> bool {
    let o = ip.octets();
    (o[0] == 169 && o[1] == 254) || (o[0] == 172 && (17..=20).contains(&o[1]))
}

/// Dial only well-known bootstrap hosts at their resolved public IPs (or relay circuit).
pub(crate) fn is_trusted_bootstrap_dial_addr(ma: &Multiaddr) -> bool {
    if is_relay_circuit_multiaddr(ma) {
        return true;
    }
    ipv4_from_ma_str(&ma.to_string()).is_some_and(is_public_bootstrap_ipv4)
}

/// Expand `0.0.0.0` / `::` listeners into concrete LAN addresses for coord/mDNS peerstore.
pub(crate) fn expand_listen_addresses(addr: &Multiaddr) -> Vec<Multiaddr> {
    let s = addr.to_string();
    if s.contains("/ip4/0.0.0.0/") {
        let mut out = Vec::new();
        if let Some(port) = s.split("/tcp/").nth(1).and_then(|p| p.split('/').next()) {
            for ip in local_ipv4_addrs() {
                let ma_str = format!("/ip4/{ip}/tcp/{port}");
                if let Ok(ma) = ma_str.parse::<Multiaddr>() {
                    out.push(ma);
                }
            }
        }
        if let Some(rest) = s.split("/udp/").nth(1) {
            let port = rest.split('/').next().unwrap_or("");
            let tail = rest.split_once('/').map(|(_, t)| t).unwrap_or("");
            for ip in local_ipv4_addrs() {
                let ma_str = format!("/ip4/{ip}/udp/{port}/{tail}");
                if let Ok(ma) = ma_str.parse::<Multiaddr>() {
                    out.push(ma);
                }
            }
        }
        if !out.is_empty() {
            return out;
        }
    }
    if is_publishable_listen_addr(addr) {
        vec![addr.clone()]
    } else {
        Vec::new()
    }
}

pub(crate) fn peer_id_from_multiaddr(ma: &Multiaddr) -> Option<PeerId> {
    ma.iter().find_map(|p| {
        if let Protocol::P2p(pid) = p {
            Some(pid)
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use libp2p::Multiaddr;

    #[test]
    fn publishable_rejects_docker_bridge_listen() {
        for ip in ["172.17.0.1", "172.18.0.1", "172.19.0.1", "172.20.0.1"] {
            let ma: Multiaddr = format!("/ip4/{ip}/tcp/41295").parse().unwrap();
            assert!(!is_publishable_listen_addr(&ma), "must reject {ip}");
        }
    }

    #[test]
    fn publishable_allows_lan_192_168() {
        let ma: Multiaddr = "/ip4/192.168.1.42/tcp/4001".parse().unwrap();
        assert!(is_publishable_listen_addr(&ma));
    }

    #[test]
    fn relay_bootnode_ip4_base_builds_dialable_circuit_addr() {
        let peer = "12D3KooWPjceQrSwdWXPyLLeABRXmuqt69Rg3sBYbU1Nft9HyQ6X";
        let nodes = resolve_relay_bootnodes(peer, &["/ip4/203.0.113.7/tcp/4002".to_string()]);
        assert_eq!(nodes.len(), 1, "public ip4 relay base should resolve to 1 addr");
        let (p, ma) = &nodes[0];
        assert_eq!(p.to_string(), peer);
        assert_eq!(ma.to_string(), format!("/ip4/203.0.113.7/tcp/4002/p2p/{peer}"));
        // The resulting addr must be trusted for dialing like any other bootstrap.
        assert!(is_trusted_bootstrap_dial_addr(ma));
    }

    #[test]
    fn relay_bootnode_rejects_private_and_bad_inputs() {
        let peer = "12D3KooWPjceQrSwdWXPyLLeABRXmuqt69Rg3sBYbU1Nft9HyQ6X";
        // RFC1918 base is not a public relay endpoint.
        assert!(resolve_relay_bootnodes(peer, &["/ip4/192.168.1.5/tcp/4002".to_string()]).is_empty());
        // Bad peer id.
        assert!(resolve_relay_bootnodes("not-a-peer", &["/ip4/203.0.113.7/tcp/4002".to_string()]).is_empty());
        // Missing tcp/port.
        assert!(resolve_relay_bootnodes(peer, &["/ip4/203.0.113.7".to_string()]).is_empty());
    }

    #[test]
    fn trusted_bootstrap_rejects_private_ip() {
        let ma: Multiaddr =
            "/ip4/172.20.0.1/tcp/41295/p2p/QmbLHAnMoJPWSCR5Zhtx6BHJX9KiKNN6tpvbUcqanj75Nb"
                .parse()
                .unwrap();
        assert!(!is_trusted_bootstrap_dial_addr(&ma));
    }

    #[test]
    fn coord_dial_ipv4_relay_before_ipv6() {
        let v6: Multiaddr = "/ip6/2001:41d0:203:2ca6::/tcp/4001/p2p/QmbLHAnMoJPWSCR5Zhtx6BHJX9KiKNN6tpvbUcqanj75Nb/p2p-circuit/p2p/16Uiu2HAm5KP74oCyKi9sfYYA9P2FtpdjZCE2WQwxH1w2FTV1Kp3P"
            .parse()
            .unwrap();
        let v4: Multiaddr = "/ip4/54.38.47.166/tcp/4001/p2p/QmbLHAnMoJPWSCR5Zhtx6BHJX9KiKNN6tpvbUcqanj75Nb/p2p-circuit/p2p/16Uiu2HAm5KP74oCyKi9sfYYA9P2FtpdjZCE2WQwxH1w2FTV1Kp3P"
            .parse()
            .unwrap();
        let out = sort_coord_dial_multiaddrs(vec![v6.clone(), v4.clone()]);
        assert_eq!(out[0], v4);
        assert_eq!(out[1], v6);
    }

    #[test]
    fn profile_wifi_with_cellular_iface_is_lan() {
        let p = LocalNetworkProfile {
            has_wifi_iface: true,
            has_rfc1918_ipv4: true,
            has_cellular_iface: true,
            ..Default::default()
        };
        assert_eq!(p.mode_label(), "lan");
        assert!(!p.avoid_blind_routed_dial());
        assert!(!p.on_mobile_data_path());
    }

    #[test]
    fn network_handover_key_changes_public_ipv4() {
        let a = LocalNetworkProfile {
            has_public_ipv4: true,
            primary_public_ipv4: Some(std::net::Ipv4Addr::new(203, 0, 113, 10)),
            ..Default::default()
        };
        let b = LocalNetworkProfile {
            has_public_ipv4: true,
            primary_public_ipv4: Some(std::net::Ipv4Addr::new(203, 0, 113, 11)),
            ..Default::default()
        };
        assert_ne!(network_handover_key(&a), network_handover_key(&b));
    }

    #[test]
    fn network_handover_key_changes_wifi_to_mobile() {
        let wifi = LocalNetworkProfile {
            has_wifi_iface: true,
            has_rfc1918_ipv4: true,
            primary_rfc1918_ipv4: Some(std::net::Ipv4Addr::new(192, 168, 1, 5)),
            ..Default::default()
        };
        let mobile = LocalNetworkProfile {
            has_cellular_iface: true,
            has_cgnat_ipv4: true,
            primary_cgnat_ipv4: Some(std::net::Ipv4Addr::new(100, 91, 13, 241)),
            ..Default::default()
        };
        assert_ne!(
            network_handover_key(&wifi),
            network_handover_key(&mobile)
        );
    }

    #[test]
    fn profile_cgnat_without_lan_is_mobile_data() {
        let p = LocalNetworkProfile {
            has_cellular_iface: true,
            has_cgnat_ipv4: true,
            ..Default::default()
        };
        assert_eq!(p.mode_label(), "mobile-data");
        assert!(p.avoid_blind_routed_dial());
        assert!(p.on_mobile_data_path());
    }

    #[test]
    fn wan_first_sort_puts_relay_before_lan() {
        let lan: Multiaddr = "/ip4/192.168.1.50/tcp/41000".parse().unwrap();
        let relay: Multiaddr = "/ip4/51.81.93.51/tcp/4001/p2p/QmQCU2EcMqAqQPR2i9bChDtGNJchTbq5TbXJJ16u19uLTa/p2p-circuit/p2p/16Uiu2HAm699TtKnm9LHXoS6MbVp8ehX7U8hyomVhivz9KuVKsYis"
            .parse()
            .unwrap();
        let out = sort_dm_dial_addrs_wan_first(vec![lan.clone(), relay.clone()]);
        assert_eq!(out[0], relay);
    }

}
