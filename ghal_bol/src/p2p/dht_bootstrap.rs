//! Public libp2p/IPFS Kademlia bootstrap + invite address seeding for WAN reachability.

use std::net::{IpAddr, ToSocketAddrs};
use std::str::FromStr;
use std::time::Duration;

use libp2p::kad;
use libp2p::multiaddr::Protocol;
use libp2p::{Multiaddr, PeerId};

use super::native_log;

/// libp2p IPFS Kademlia bootstrap peers — each has its own DNS host (see `_dnsaddr.bootstrap.libp2p.io`).
const PUBLIC_DHT_BOOTSTRAP: [(&str, &str); 4] = [
    (
        "QmNnooDu7bfjPFoTZYxMNLWUQJyrVwtbZg5gBMjTezGAJN",
        "sv15.bootstrap.libp2p.io",
    ),
    (
        "QmQCU2EcMqAqQPR2i9bChDtGNJchTbq5TbXJJ16u19uLTa",
        "ny5.bootstrap.libp2p.io",
    ),
    (
        "QmbLHAnMoJPWSCR5Zhtx6BHJX9KiKNN6tpvbUcqanj75Nb",
        "am6.bootstrap.libp2p.io",
    ),
    (
        "QmcZf59bWwK5XFi76CZX8cbJ4BhTzzA3gU1ZjYZcYW3dwt",
        "sg1.bootstrap.libp2p.io",
    ),
];

const BOOTSTRAP_TCP_PORT: u16 = 4001;

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
}

/// Detects Wi‑Fi ↔ mobile and CGNAT IP changes (same mode, new carrier address).
pub(crate) fn network_handover_key(p: &LocalNetworkProfile) -> NetworkHandoverKey {
    NetworkHandoverKey {
        active_lan: p.has_active_lan(),
        mobile_path: p.on_mobile_data_path(),
        mode: p.mode_label(),
        cgnat: p.primary_cgnat_ipv4,
        lan_v4: p.primary_rfc1918_ipv4,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NetworkHandoverKey {
    pub active_lan: bool,
    pub mobile_path: bool,
    pub mode: &'static str,
    pub cgnat: Option<std::net::Ipv4Addr>,
    pub lan_v4: Option<std::net::Ipv4Addr>,
}

impl LocalNetworkProfile {
    /// Wi‑Fi or tether LAN is active — prefer mDNS/direct TCP; do not treat as cellular-only.
    pub(crate) fn has_active_lan(&self) -> bool {
        if self.has_rfc1918_ipv4
            && (self.has_wifi_iface || self.has_tether_iface || self.has_usb_iface)
        {
            return true;
        }
        false
    }

    /// Skip blind `DialOpts::peer_id` dials; use coord/KAD/mDNS explicit multiaddrs instead.
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
        "use coord + DHT + mDNS"
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

/// Resolve bootstrap DNS via the OS resolver and dial `/ip4/.../tcp/4001/p2p/<peer>`.
/// Avoids libp2p `/dns/...` returning docker-bridge IPs (e.g. 172.17.0.1 → "Unexpected peer ID").
fn public_dht_bootstrap_resolved_ip_multiaddrs() -> Vec<(PeerId, Multiaddr)> {
    let mut out = Vec::new();
    for (peer_str, host) in &PUBLIC_DHT_BOOTSTRAP {
        let Ok(peer) = PeerId::from_str(peer_str) else {
            continue;
        };
        let target = format!("{host}:{BOOTSTRAP_TCP_PORT}");
        let Ok(addrs) = target.to_socket_addrs() else {
            native_log::warn("kad", format!("bootstrap DNS resolve failed: {host}"));
            continue;
        };
        let mut pushed = false;
        for addr in addrs {
            let IpAddr::V4(ip) = addr.ip() else {
                continue;
            };
            if !is_public_bootstrap_ipv4(ip) {
                native_log::warn(
                    "kad",
                    format!("bootstrap {host} skipped non-public {ip} (docker/LAN DNS?)"),
                );
                continue;
            }
            let ma_str = format!("/ip4/{ip}/tcp/{BOOTSTRAP_TCP_PORT}/p2p/{peer}");
            let Ok(ma) = ma_str.parse::<Multiaddr>() else {
                continue;
            };
            native_log::debug("kad", format!("bootstrap {peer} via {ma}"));
            out.push((peer, ma));
            pushed = true;
        }
        if !pushed {
            native_log::warn("kad", format!("bootstrap DNS resolve: no public IPv4 for {host}"));
        }
    }
    out
}

/// Bootstrap list for Kademlia seeding + dial (async hook kept for call sites).
pub(crate) async fn resolve_public_dht_bootnodes() -> Vec<(PeerId, Multiaddr)> {
    let out = public_dht_bootstrap_resolved_ip_multiaddrs();

    native_log::info(
        "kad",
        format!(
            "public DHT bootstrap: {} dial address(es) for {} peer(s)",
            out.len(),
            PUBLIC_DHT_BOOTSTRAP.len()
        ),
    );
    out
}

pub(crate) type KadBehaviour = kad::Behaviour<kad::store::MemoryStore>;

pub(crate) fn new_kademlia_behaviour(local_peer_id: PeerId) -> KadBehaviour {
    let mut cfg = kad::Config::new(kad::PROTOCOL_NAME);
    cfg.set_query_timeout(Duration::from_secs(60));
    let store = kad::store::MemoryStore::new(local_peer_id);
    let mut kad = kad::Behaviour::with_config(local_peer_id, store, cfg);
    // Client: query the public DHT without maintaining connections to every routing-table peer.
    kad.set_mode(Some(kad::Mode::Client));
    kad
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
pub(crate) fn is_kad_publish_tcp_multiaddr(ma: &Multiaddr) -> bool {
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
        .filter(|ma| is_kad_publish_tcp_multiaddr(ma))
        .collect()
}

/// DHT publish when a coord URL is set: relay circuit + public TCP only (never CGNAT/LAN).
pub(crate) fn tcp_dm_publish_addrs_coord_mode(addrs: Vec<Multiaddr>) -> Vec<Multiaddr> {
    addrs
        .into_iter()
        .filter(|ma| {
            is_coord_relay_tcp_circuit_multiaddr(ma) || is_coord_register_tcp_multiaddr(ma)
        })
        .collect()
}

/// Inbound DHT records to dial when coord is primary (WAN paths only).
pub(crate) fn wan_kad_record_dial_addrs(addrs: Vec<Multiaddr>) -> Vec<Multiaddr> {
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

/// When a routable public IP is present, skip RFC1918 LAN (coord/DHT often list both).
/// LAN + relay only: keep both — try direct TCP before relay circuit.
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
    if !is_kad_publish_tcp_multiaddr(ma) || is_relay_circuit_multiaddr(ma) {
        return false;
    }
    ipv4_from_ma_str(&ma.to_string())
        .map(is_public_bootstrap_ipv4)
        .unwrap_or(false)
}

/// Legacy DHT-style filter: public TCP + relay only (drops RFC1918/CGNAT).
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

/// Dial addrs from GET /v1/peers — use what the peer registered (CGNAT, LAN, relay).
/// Skips loopback and docker bridge junk only; WAN-prefers when a true public IP exists.
pub(crate) fn filter_coord_presence_dial_addrs(addrs: Vec<Multiaddr>) -> Vec<Multiaddr> {
    let sorted = sort_dm_dial_addrs(addrs);
    let filtered: Vec<Multiaddr> = sorted
        .into_iter()
        .filter(|ma| {
            if !is_dm_dial_multiaddr(ma) {
                return false;
            }
            if is_relay_circuit_multiaddr(ma) {
                return true;
            }
            if let Some(ip) = ipv4_from_ma_str(&ma.to_string()) {
                return !ip.is_loopback() && !is_docker_or_link_local_ipv4(ip);
            }
            true
        })
        .collect();
    filtered
}

pub(crate) fn kad_publish_fingerprint(addrs: &[Multiaddr]) -> String {
    let mut lines: Vec<String> = addrs.iter().map(|a| a.to_string()).collect();
    lines.sort();
    lines.join("\n")
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

/// docker0 / podman / common CNI bridges — not reachable for DM or DHT (see user logs: 172.17–172.20).
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

/// Expand `0.0.0.0` / `::` listeners into concrete LAN addresses for DHT + peerstore.
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

const ADDR_RECORD_PREFIX: &[u8] = b"ghal_bol_peer_addrs_v1\n";

pub(crate) fn kad_peer_record_key(peer: &PeerId) -> kad::RecordKey {
    kad::RecordKey::new(&peer.to_bytes())
}

fn encode_addr_record(addrs: &[Multiaddr]) -> Vec<u8> {
    let mut v = ADDR_RECORD_PREFIX.to_vec();
    for ma in addrs {
        v.extend_from_slice(ma.to_string().as_bytes());
        v.push(b'\n');
    }
    v
}

pub(crate) fn decode_addr_record(bytes: &[u8]) -> Vec<Multiaddr> {
    if !bytes.starts_with(ADDR_RECORD_PREFIX) {
        return Vec::new();
    }
    let text = match std::str::from_utf8(&bytes[ADDR_RECORD_PREFIX.len()..]) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .filter_map(|l| l.parse::<Multiaddr>().ok())
        .filter(|ma| is_dm_dial_multiaddr(ma))
        .collect()
}

pub(crate) fn peer_id_from_record_key(key: &kad::RecordKey) -> Option<PeerId> {
    PeerId::from_bytes(key.as_ref()).ok()
}

/// Publish dialable multiaddrs for [peer] (DHT record + provider).
pub(crate) fn kad_publish_peer_record(kad: &mut KadBehaviour, peer: &PeerId, addrs: Vec<Multiaddr>) {
    let addrs = tcp_dm_publish_addrs(addrs);
    if addrs.is_empty() {
        return;
    }
    for ma in &addrs {
        kad.add_address(peer, ma.clone());
    }
    let key = kad_peer_record_key(peer);
    let record = kad::Record::new(key.clone(), encode_addr_record(&addrs));
    match kad.put_record(record, kad::Quorum::One) {
        Ok(_) => native_log::info(
            "kad",
            format!("put_record {peer} ({} tcp addr(s))", addrs.len()),
        ),
        Err(e) => native_log::warn("kad", format!("put_record {peer}: {e}")),
    }
    if let Err(e) = kad.start_providing(key) {
        native_log::debug("kad", format!("start_providing {peer}: {e}"));
    }
}

/// Walk the DHT toward [peer].
pub(crate) fn kad_find_peer(kad: &mut KadBehaviour, peer: PeerId) {
    kad.get_closest_peers(peer);
}

/// Secondary hints (custom addr record + provider); not relied on for first connect.
pub(crate) fn kad_lookup_peer(kad: &mut KadBehaviour, peer: PeerId) {
    kad_find_peer(kad, peer);
    let key = kad_peer_record_key(&peer);
    kad.get_providers(key.clone());
    kad.get_record(key);
}

/// Seed the routing table with public DHT bootnodes and dial hints from connect invites.
pub(crate) fn seed_kad_routing_table(
    kad: &mut KadBehaviour,
    invite_bootstrap: &[Multiaddr],
    resolved_public: &[(PeerId, Multiaddr)],
) {
    for (peer, addr) in resolved_public {
        kad.add_address(peer, addr.clone());
    }

    for ma in invite_bootstrap {
        if ma.is_empty() {
            continue;
        }
        let Some(peer) = peer_id_from_multiaddr(ma) else {
            continue;
        };
        kad.add_address(&peer, ma.clone());
    }
}

/// Join the public DHT (no-op if routing table has no known peers yet).
pub(crate) fn bootstrap_kad(kad: &mut KadBehaviour) {
    if let Err(e) = kad.bootstrap() {
        native_log::info("kad", format!("kad bootstrap skipped: {e}"));
    } else {
        native_log::info("kad", "kad bootstrap started");
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
    fn coord_mode_publish_skips_cgnat_keeps_relay() {
        let cgnat: Multiaddr = "/ip4/100.104.255.165/tcp/40993".parse().unwrap();
        let relay: Multiaddr = "/ip4/51.81.93.51/tcp/4001/p2p/QmQCU2EcMqAqQPR2i9bChDtGNJchTbq5TbXJJ16u19uLTa/p2p-circuit/p2p/16Uiu2HAm699TtKnm9LHXoS6MbVp8ehX7U8hyomVhivz9KuVKsYis"
            .parse()
            .unwrap();
        let out = tcp_dm_publish_addrs_coord_mode(vec![cgnat, relay.clone()]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0], relay);
    }
}
