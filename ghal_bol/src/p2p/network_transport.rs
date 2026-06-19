//! Network profile detection, relay/coord dial helpers, and listen-address utilities (no Kademlia).

use std::collections::HashSet;
use std::net::{IpAddr, ToSocketAddrs};
use std::str::FromStr;

use libp2p::multiaddr::Protocol;
use libp2p::{Multiaddr, PeerId};

use super::native_log;

/// OS-reported default network transport (ConnectivityManager / Linux default route).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum OsDefaultTransport {
    #[default]
    None,
    Wifi,
    Cellular,
    Ethernet,
}

/// Authoritative OS network snapshot — updated on connectivity callbacks + each `network_tick`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct OsNetworkSnapshot {
    pub default_transport: OsDefaultTransport,
    /// Android `NET_CAPABILITY_VALIDATED`; Linux: default route iface operstate up.
    pub internet_validated: bool,
    /// Android `NET_CAPABILITY_INTERNET` on the default network.
    pub has_internet: bool,
    /// Wi‑Fi link up (Android any-WiFi-network or Linux `wl*` operstate).
    pub wifi_link_up: bool,
    /// Linux: iface name carrying the default IPv4 route (e.g. `wlan0`, `eth0`).
    pub default_route_iface: Option<String>,
}

static OS_NETWORK_SNAPSHOT: std::sync::OnceLock<std::sync::RwLock<OsNetworkSnapshot>> =
    std::sync::OnceLock::new();

fn os_network_mx() -> &'static std::sync::RwLock<OsNetworkSnapshot> {
    OS_NETWORK_SNAPSHOT.get_or_init(|| std::sync::RwLock::new(OsNetworkSnapshot::default()))
}

/// Probe OS connectivity and cache — call on Android notify + each `network_tick`.
pub(crate) fn refresh_os_network_truth() {
    let snap = probe_os_network_truth_platform();
    if let Ok(mut g) = os_network_mx().write() {
        *g = snap;
    }
}

pub(crate) fn os_network_snapshot() -> OsNetworkSnapshot {
    os_network_mx()
        .read()
        .ok()
        .map(|g| g.clone())
        .unwrap_or_default()
}

fn probe_os_network_truth_platform() -> OsNetworkSnapshot {
    #[cfg(target_os = "android")]
    {
        return crate::android_network::probe_connectivity_truth();
    }
    #[cfg(target_os = "linux")]
    {
        return crate::linux_network::probe_connectivity_truth();
    }
    #[cfg(not(any(target_os = "android", target_os = "linux")))]
    {
        OsNetworkSnapshot::default()
    }
}

/// Fresh OS probe for the **Flutter UI process** (not the `:p2p` / daemon cached snapshot).
pub(crate) fn probe_os_network_truth_ui() -> Option<OsNetworkSnapshot> {
    #[cfg(target_os = "linux")]
    {
        return Some(crate::linux_network::probe_connectivity_truth());
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

pub(crate) fn os_default_transport_label(t: OsDefaultTransport) -> &'static str {
    match t {
        OsDefaultTransport::Wifi => "wifi",
        OsDefaultTransport::Cellular => "cell",
        OsDefaultTransport::Ethernet => "ethernet",
        OsDefaultTransport::None => "none",
    }
}

/// JSON for Flutter `NetworkHelper` — mirrors `Native/flow` `os=` fields (display only).
pub(crate) fn os_network_snapshot_to_json(
    snap: &OsNetworkSnapshot,
    source: &str,
) -> serde_json::Value {
    serde_json::json!({
        "ok": true,
        "source": source,
        "default_transport": os_default_transport_label(snap.default_transport),
        "internet_validated": snap.internet_validated,
        "has_internet": snap.has_internet,
        "wifi_link_up": snap.wifi_link_up,
        "default_route_iface": snap.default_route_iface,
    })
}

/// Authoritative OS snapshot for UI (daemon RPC / in-process FFI).
/// `:p2p` `network_tick` owns probes — UI RPC reads the cached snapshot only.
pub(crate) fn network_snapshot_for_ui(source: &str) -> serde_json::Value {
    if source == "ffi" {
        refresh_os_network_truth();
    }
    let snap = os_network_snapshot();
    log_ui_network_if_changed(&snap, source);
    os_network_snapshot_to_json(&snap, source)
}

fn log_ui_network_if_changed(snap: &OsNetworkSnapshot, source: &str) {
    use std::sync::Mutex;
    static LAST: std::sync::OnceLock<Mutex<Option<(OsDefaultTransport, bool, bool, bool)>>> =
        std::sync::OnceLock::new();
    let key = (
        snap.default_transport,
        snap.internet_validated,
        snap.has_internet,
        snap.wifi_link_up,
    );
    let mx = LAST.get_or_init(|| Mutex::new(None));
    let Ok(mut g) = mx.lock() else {
        return;
    };
    let changed = g.map(|prev| prev != key).unwrap_or(true);
    if !changed {
        return;
    }
    *g = Some(key);
    let route = snap
        .default_route_iface
        .as_deref()
        .unwrap_or("-");
    native_log::info(
        "network",
        format!(
            "ui snapshot source={source} os={}/validated={}/wifi={} route={route} internet={}",
            os_default_transport_label(snap.default_transport),
            snap.internet_validated,
            if snap.wifi_link_up { "up" } else { "down" },
            snap.has_internet,
        ),
    );
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
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
    /// RFC1918 on a Wi‑Fi-named interface — LAN even when rmnet/CGNAT stays visible.
    pub has_rfc1918_on_wifi: bool,
    /// Direct public IPv4 on an interface (VPS / rare home WAN).
    pub primary_public_ipv4: Option<std::net::Ipv4Addr>,
    /// Cached OS truth merged on each profile detect (see `merge_os_network_truth`).
    pub os: OsNetworkSnapshot,
}

/// Detects Wi‑Fi ↔ mobile, CGNAT/LAN IP changes, and direct public IPv4 churn.
pub(crate) fn network_handover_key(p: &LocalNetworkProfile) -> NetworkHandoverKey {
    NetworkHandoverKey {
        active_lan: p.has_active_lan(),
        rfc1918_on_wifi: p.has_rfc1918_on_wifi,
        mobile_path: p.on_mobile_data_path(),
        mode: p.mode_label(),
        os_default: p.os.default_transport,
        os_validated: p.os.internet_validated,
        cgnat: p.primary_cgnat_ipv4,
        lan_v4: p.primary_rfc1918_ipv4,
        public_v4: p.primary_public_ipv4,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NetworkHandoverKey {
    pub active_lan: bool,
    pub rfc1918_on_wifi: bool,
    pub mobile_path: bool,
    pub mode: &'static str,
    pub os_default: OsDefaultTransport,
    pub os_validated: bool,
    pub cgnat: Option<std::net::Ipv4Addr>,
    pub lan_v4: Option<std::net::Ipv4Addr>,
    pub public_v4: Option<std::net::Ipv4Addr>,
}

/// Relay circuit listen addr we register on coord and fingerprint (IPv4/dns4 only).
/// libp2p may also open a `/dns6/.../p2p-circuit` listener; treating that as a WAN
/// path change clears bootstrap HOP state and invalidates the live reservation.
pub(crate) fn is_coord_ipv4_relay_listen(ma: &Multiaddr) -> bool {
    if !is_coord_relay_tcp_circuit_multiaddr(ma) {
        return false;
    }
    let s = ma.to_string();
    s.contains("/ip4/") || s.contains("/dns4/")
}

/// Sorted WAN-relevant listen addrs (relay circuit + public TCP) for drift detection.
pub(crate) fn wan_coord_listen_fingerprint(addrs: &[Multiaddr]) -> Vec<String> {
    let mut keys: Vec<String> = addrs
        .iter()
        .filter(|ma| is_coord_ipv4_relay_listen(ma) || is_coord_register_tcp_multiaddr(ma))
        .map(|ma| ma.to_string())
        .collect();
    keys.sort();
    keys.dedup();
    keys
}

impl LocalNetworkProfile {
    /// OS default network is cellular with no Wi‑Fi/Ethernet default — truth for mobile-data path.
    pub(crate) fn os_on_cellular_default(&self) -> bool {
        matches!(self.os.default_transport, OsDefaultTransport::Cellular)
    }

    /// OS default network is Wi‑Fi or Ethernet.
    pub(crate) fn os_on_lan_default(&self) -> bool {
        matches!(
            self.os.default_transport,
            OsDefaultTransport::Wifi | OsDefaultTransport::Ethernet
        )
    }

    /// Wi‑Fi, tether, or wired LAN — prefer mDNS/direct TCP; do not treat as cellular-only.
    pub(crate) fn has_active_lan(&self) -> bool {
        // OS default route is cellular — not on LAN even when rmnet/CGNAT iface lingers in if_addrs.
        if self.os_on_cellular_default() {
            return false;
        }
        if self.os_on_lan_default() && self.os.wifi_link_up {
            return true;
        }
        if self.has_rfc1918_on_wifi {
            return true;
        }
        if self.has_rfc1918_ipv4
            && (self.has_wifi_iface || self.has_tether_iface || self.has_usb_iface)
        {
            return true;
        }
        // Desktop wired Ethernet (and similar): RFC1918 without cellular/CGNAT-only path.
        if self.has_rfc1918_ipv4 && !self.has_cellular_iface && !self.has_cgnat_ipv4 {
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
        if self.on_mobile_data_path() {
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
        if self.on_mobile_data_path() {
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

    /// Compact OS truth for `Native/flow` connectivity snapshots.
    pub(crate) fn os_truth_label(&self) -> String {
        let transport = match self.os.default_transport {
            OsDefaultTransport::Wifi => "wifi",
            OsDefaultTransport::Cellular => "cell",
            OsDefaultTransport::Ethernet => "eth",
            OsDefaultTransport::None => "none",
        };
        let validated = if self.os.internet_validated {
            "validated"
        } else {
            "unvalidated"
        };
        let wifi = if self.os.wifi_link_up { "wifi_up" } else { "wifi_down" };
        if let Some(iface) = &self.os.default_route_iface {
            format!("os={transport}/{validated}/{wifi} route={iface}")
        } else {
            format!("os={transport}/{validated}/{wifi}")
        }
    }

    /// Cellular/CGNAT without an active LAN — needs relay for coord when URL is set.
    pub(crate) fn on_mobile_data_path(&self) -> bool {
        if self.os_on_cellular_default() {
            return true;
        }
        !self.has_active_lan() && (self.has_cellular_iface || self.has_cgnat_ipv4)
    }

    /// CGNAT / no public IPv4 — WAN peers need a relay circuit even on Wi‑Fi LAN.
    pub(crate) fn needs_relay_for_wan(&self) -> bool {
        !self.has_public_ipv4 && (self.on_mobile_data_path() || self.has_cgnat_ipv4)
    }
}

/// Android Wi‑Fi return can lag `if_addrs` while libp2p is already listening on RFC1918 TCP.
/// Only promote to LAN when the OS also reports Wi‑Fi linked — not from listen addr alone.
pub(crate) fn effective_network_profile(
    detected: LocalNetworkProfile,
    has_rfc1918_dm_listen: bool,
    platform_wifi_linked: bool,
) -> LocalNetworkProfile {
    if detected.has_active_lan() || !has_rfc1918_dm_listen || !platform_wifi_linked {
        return detected;
    }
    let mut p = detected;
    p.has_rfc1918_ipv4 = true;
    p.has_rfc1918_on_wifi = true;
    p
}

/// Merge cached OS connectivity truth into an `if_addrs`-derived profile.
pub(crate) fn merge_os_network_truth(p: &mut LocalNetworkProfile) {
    p.os = os_network_snapshot();
    if p.os.wifi_link_up {
        p.has_wifi_iface = true;
    }
    if p.os_on_lan_default() {
        if p.has_rfc1918_ipv4 || p.os.default_transport == OsDefaultTransport::Wifi {
            p.has_rfc1918_on_wifi = true;
        }
    }
    if p.os_on_cellular_default() {
        p.has_rfc1918_on_wifi = false;
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
        let is_wifi = iface_name_is_wifi(name);
        if is_wifi {
            p.has_wifi_iface = true;
        }
        if iface_name_is_cellular(name) {
            p.has_cellular_iface = true;
        }
        if iface_name_is_tether(name) {
            p.has_tether_iface = true;
            if name.to_ascii_lowercase().contains("usb")
                || name.to_ascii_lowercase().contains("rndis")
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
                    if is_wifi {
                        p.has_rfc1918_on_wifi = true;
                    }
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
    merge_os_network_truth(&mut p);
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
pub(crate) fn resolve_relay_bootnodes(
    peer_str: &str,
    addrs: &[String],
) -> Vec<(PeerId, Multiaddr)> {
    let Ok(peer) = PeerId::from_str(peer_str) else {
        native_log::warn("relay", format!("ghalbol relay bad peer id: {peer_str}"));
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for addr in addrs {
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
    let mut host: Option<(String, bool, bool)> = None;
    let mut port: Option<u16> = None;
    let mut i = 0;
    while i + 1 < segs.len() {
        match segs[i] {
            "ip4" => host = Some((segs[i + 1].to_string(), false, false)),
            "ip6" => host = Some((segs[i + 1].to_string(), false, true)),
            "dns4" | "dns" => host = Some((segs[i + 1].to_string(), true, false)),
            "dns6" => host = Some((segs[i + 1].to_string(), true, true)),
            "tcp" => port = segs[i + 1].parse().ok(),
            _ => {}
        }
        i += 1;
    }
    let (Some((h, is_dns, is_v6)), Some(p)) = (host, port) else {
        native_log::warn(
            "relay",
            format!("ghalbol relay addr not TCP host/port: {addr}"),
        );
        return Vec::new();
    };
    let mut out = Vec::new();
    if is_dns {
        let Ok(resolved) = format!("{h}:{p}").to_socket_addrs() else {
            native_log::warn("relay", format!("ghalbol relay DNS resolve failed: {h}"));
            return Vec::new();
        };
        for sa in resolved {
            match sa.ip() {
                IpAddr::V4(ip) if !is_v6 => {
                    if !is_public_bootstrap_ipv4(ip) {
                        continue;
                    }
                    if let Ok(ma) = format!("/ip4/{ip}/tcp/{p}/p2p/{peer}").parse::<Multiaddr>() {
                        out.push(ma);
                    }
                }
                IpAddr::V6(ip) if is_v6 && !ip.is_loopback() && !ip.is_unspecified() => {
                    if let Ok(ma) = format!("/ip6/{ip}/tcp/{p}/p2p/{peer}").parse::<Multiaddr>() {
                        out.push(ma);
                    }
                }
                _ => {}
            }
        }
    } else if is_v6 {
        if let Ok(ip) = h.parse::<std::net::Ipv6Addr>() {
            if !ip.is_loopback() && !ip.is_unspecified() {
                if let Ok(ma) = format!("/ip6/{ip}/tcp/{p}/p2p/{peer}").parse::<Multiaddr>() {
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

/// Append `/p2p-circuit` to the connected bootstrap TCP multiaddr (preserves live port).
pub(crate) fn relay_circuit_listen_addr(base: &Multiaddr) -> Option<Multiaddr> {
    if is_relay_circuit_multiaddr(base) {
        return Some(base.clone());
    }
    let mut ma = base.clone();
    if !ma.iter().any(|p| matches!(p, Protocol::P2pCircuit)) {
        ma.push(Protocol::P2pCircuit);
    }
    Some(ma)
}

/// Lower rank = preferred bootstrap TCP family for this profile.
pub(crate) fn relay_bootstrap_family_rank(ma: &Multiaddr, mobile: bool, ipv6_degraded: bool) -> u8 {
    let s = ma.to_string();
    if s.contains("/ip4/") {
        if mobile || ipv6_degraded { 0 } else { 1 }
    } else if s.contains("/ip6/") {
        if mobile || ipv6_degraded { 1 } else { 0 }
    } else {
        2
    }
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
    if s.contains("/ip6/::1/") || s.contains("/ip6/fe80:") {
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
/// WAN = `/p2p-circuit` only — bare public TCP from coord is often relay bootstrap, not the peer.
pub(crate) fn wan_coord_dial_addrs(addrs: Vec<Multiaddr>) -> Vec<Multiaddr> {
    let filtered: Vec<Multiaddr> = addrs
        .into_iter()
        .filter(|ma| {
            is_coord_relay_tcp_circuit_multiaddr(ma)
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
        ipv4_from_ma_str(&ma.to_string()).is_some_and(|ip| !ip.is_private() && !ip.is_loopback())
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

/// Host:port keys (`159.223.110.159:28048`) from `GET /v1/relay` base multiaddrs.
pub(crate) fn relay_bootstrap_tcp_keys(addrs: &[String]) -> HashSet<String> {
    let mut out = HashSet::new();
    for addr in addrs {
        if let Some((host, port)) = relay_base_tcp_host_port(addr) {
            out.insert(format!("{host}:{port}"));
        }
    }
    out
}

/// Parse `/ip4/<host>/tcp/<port>` from a relay bootstrap base multiaddr.
pub(crate) fn relay_base_tcp_host_port(addr: &str) -> Option<(String, u16)> {
    let s = addr.trim();
    let host = s.split("/ip4/").nth(1)?.split('/').next()?.to_string();
    let port = s
        .split("/tcp/")
        .nth(1)?
        .split('/')
        .next()?
        .parse()
        .ok()?;
    Some((host, port))
}

pub(crate) fn is_relay_bootstrap_tcp(
    host: &str,
    port: u16,
    bootstraps: &HashSet<String>,
) -> bool {
    bootstraps.contains(&format!("{}:{port}", host.trim()))
}

/// Last `/p2p/<id>` on a multiaddr (local peer on DM listen, client peer on relay circuit).
pub(crate) fn terminal_p2p_peer_id(ma: &Multiaddr) -> Option<PeerId> {
    let mut last = None;
    for p in ma.iter() {
        if let Protocol::P2p(pid) = p {
            last = Some(pid);
        }
    }
    last
}

/// Coord registration: public routable TCP only (not RFC1918 — mDNS covers LAN per DESIGN.md).
/// Must include `/p2p/<local_peer>` — bare `/ip4/relay-host/tcp/port` bootstrap hops are not DM listens.
pub(crate) fn is_coord_register_tcp_multiaddr(ma: &Multiaddr) -> bool {
    if !is_dm_listen_tcp_multiaddr(ma) || is_relay_circuit_multiaddr(ma) {
        return false;
    }
    let s = ma.to_string();
    if !s.contains("/p2p/") {
        return false;
    }
    ipv4_from_ma_str(&s)
        .map(is_public_bootstrap_ipv4)
        .unwrap_or(false)
}

/// POST /v1/register TCP: peer's own public DM listen — not relay bootstrap or relay hop.
pub(crate) fn is_peer_own_coord_register_tcp(
    ma: &Multiaddr,
    local_peer_id: &str,
    relay_bootstraps: &HashSet<String>,
    preferred_relay_peer_id: Option<&str>,
) -> bool {
    if !is_coord_register_tcp_multiaddr(ma) {
        return false;
    }
    let terminal = terminal_p2p_peer_id(ma)
        .map(|p| p.to_string())
        .unwrap_or_default();
    if terminal != local_peer_id {
        return false;
    }
    let s = ma.to_string();
    if let Some(relay) = preferred_relay_peer_id {
        if s.contains(&format!("/p2p/{relay}")) {
            return false;
        }
    }
    if let Some(ip) = ipv4_from_ma_str(&s) {
        if let Some(port) = s
            .split("/tcp/")
            .nth(1)
            .and_then(|p| p.split('/').next())
            .and_then(|p| p.parse::<u16>().ok())
        {
            if is_relay_bootstrap_tcp(&ip.to_string(), port, relay_bootstraps) {
                return false;
            }
        }
    }
    true
}

/// WAN coord dial filter: `/p2p-circuit` only (bare public TCP is often relay bootstrap, not peer DM).
pub(crate) fn filter_coord_dial_addrs(addrs: Vec<Multiaddr>) -> Vec<Multiaddr> {
    sort_coord_dial_multiaddrs(
        addrs
            .into_iter()
            .filter(|ma| is_relay_circuit_multiaddr(ma))
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

/// WAN coord dials: IPv4/dns4 TCP relay circuits only (DM transport is TCP; IPv6 relay often fails).
pub(crate) fn filter_coord_relay_dial_platform(mut addrs: Vec<Multiaddr>) -> Vec<Multiaddr> {
    addrs.retain(|ma| !is_relay_circuit_multiaddr(ma) || is_coord_relay_tcp_circuit_multiaddr(ma));
    addrs.retain(|ma| {
        if !is_relay_circuit_multiaddr(ma) {
            return true;
        }
        let s = ma.to_string();
        s.contains("/ip4/") || s.contains("/dns4/")
    });
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
pub(crate) fn is_docker_or_link_local_ipv4(ip: std::net::Ipv4Addr) -> bool {
    let o = ip.octets();
    (o[0] == 169 && o[1] == 254) || (o[0] == 172 && (17..=20).contains(&o[1]))
}

/// Dial only well-known bootstrap hosts at their resolved public IPs (or relay circuit).
pub(crate) fn is_trusted_bootstrap_dial_addr(ma: &Multiaddr) -> bool {
    if is_relay_circuit_multiaddr(ma) {
        return true;
    }
    let s = ma.to_string();
    if let Some(ip) = ipv4_from_ma_str(&s) {
        return is_public_bootstrap_ipv4(ip);
    }
    if let Some(host) = s.split("/ip6/").nth(1) {
        let ip_str = host.split('/').next().unwrap_or("");
        if let Ok(ip) = ip_str.parse::<std::net::Ipv6Addr>() {
            return !ip.is_loopback() && !ip.is_unspecified();
        }
    }
    false
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
    fn peer_own_coord_register_rejects_relay_bootstrap_hop() {
        let local = "16Uiu2HAm5zdGNzac9hYfCNQZTnANbxWytcMty9twy7u942fT7MCk";
        let relay = "12D3KooWKUiRKKzspUQShSLWVwxgp1HnSAs3EgDLCQbXn5iGHGhF";
        let bootstraps = relay_bootstrap_tcp_keys(&["/ip4/159.223.110.159/tcp/28048".to_string()]);
        // Bootstrap hop accidentally ending with local peer id must still be rejected.
        let hop: Multiaddr = format!("/ip4/159.223.110.159/tcp/28048/p2p/{relay}/p2p/{local}")
            .parse()
            .unwrap();
        assert!(!is_peer_own_coord_register_tcp(
            &hop,
            local,
            &bootstraps,
            Some(relay),
        ));
        let own_public: Multiaddr = format!("/ip4/203.0.113.50/tcp/41234/p2p/{local}")
            .parse()
            .unwrap();
        assert!(is_peer_own_coord_register_tcp(
            &own_public,
            local,
            &bootstraps,
            Some(relay),
        ));
        let bootstrap_as_own: Multiaddr =
            format!("/ip4/159.223.110.159/tcp/28048/p2p/{local}").parse().unwrap();
        assert!(!is_peer_own_coord_register_tcp(
            &bootstrap_as_own,
            local,
            &bootstraps,
            Some(relay),
        ));
    }

    #[test]
    fn relay_bootstrap_tcp_keys_parses_ip4_base() {
        let keys = relay_bootstrap_tcp_keys(&["/ip4/159.223.110.159/tcp/28048".to_string()]);
        assert!(keys.contains("159.223.110.159:28048"));
    }

    #[test]
    fn publishable_rejects_docker_bridge_listen() {
        for ip in ["172.17.0.1", "172.18.0.1", "172.19.0.1", "172.20.0.1"] {
            let ma: Multiaddr = format!("/ip4/{ip}/tcp/41295").parse().unwrap();
            assert!(!is_publishable_listen_addr(&ma), "must reject {ip}");
        }
    }

    #[test]
    fn relay_bootnode_ip4_base_builds_dialable_circuit_addr() {
        let peer = "12D3KooWPjceQrSwdWXPyLLeABRXmuqt69Rg3sBYbU1Nft9HyQ6X";
        let nodes = resolve_relay_bootnodes(peer, &["/ip4/203.0.113.7/tcp/4002".to_string()]);
        assert_eq!(
            nodes.len(),
            1,
            "public ip4 relay base should resolve to 1 addr"
        );
        let (p, ma) = &nodes[0];
        assert_eq!(p.to_string(), peer);
        assert_eq!(
            ma.to_string(),
            format!("/ip4/203.0.113.7/tcp/4002/p2p/{peer}")
        );
        // The resulting addr must be trusted for dialing like any other bootstrap.
        assert!(is_trusted_bootstrap_dial_addr(ma));
    }

    #[test]
    fn relay_bootnode_rejects_private_and_bad_inputs() {
        let peer = "12D3KooWPjceQrSwdWXPyLLeABRXmuqt69Rg3sBYbU1Nft9HyQ6X";
        // RFC1918 base is not a public relay endpoint.
        assert!(
            resolve_relay_bootnodes(peer, &["/ip4/192.168.1.5/tcp/4002".to_string()]).is_empty()
        );
        // Bad peer id.
        assert!(
            resolve_relay_bootnodes("not-a-peer", &["/ip4/203.0.113.7/tcp/4002".to_string()])
                .is_empty()
        );
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
    fn wan_listen_fingerprint_ignores_dns6_relay() {
        let dns4: Multiaddr = "/dns4/coord.ghalbol.com/tcp/4002/p2p/12D3KooWKUiRKKzspUQShSLWVwxgp1HnSAs3EgDLCQbXn5iGHGhF/p2p-circuit/p2p/16Uiu2HAm5zdGNzac9hYfCNQZTnANbxWytcMty9twy7u942fT7MCk"
            .parse()
            .unwrap();
        let dns6: Multiaddr = "/dns6/coord.ghalbol.com/tcp/4002/p2p/12D3KooWKUiRKKzspUQShSLWVwxgp1HnSAs3EgDLCQbXn5iGHGhF/p2p-circuit/p2p/16Uiu2HAm5zdGNzac9hYfCNQZTnANbxWytcMty9twy7u942fT7MCk"
            .parse()
            .unwrap();
        let fp = wan_coord_listen_fingerprint(&[dns6, dns4.clone()]);
        assert_eq!(fp, vec![dns4.to_string()]);
    }

    #[test]
    fn wan_coord_dial_addrs_drop_ipv6_relay_on_all_platforms() {
        let v6: Multiaddr = "/ip6/2600:1900:4000:8dad::/tcp/4002/p2p/12D3KooWKUiRKKzspUQShSLWVwxgp1HnSAs3EgDLCQbXn5iGHGhF/p2p-circuit/p2p/16Uiu2HAm5HuMtmkgC6yPqq6g8NSrgqjbamQ6Vj6r3GjjrzKAs2Eu"
            .parse()
            .unwrap();
        let v4: Multiaddr = "/ip4/34.30.211.249/tcp/4002/p2p/12D3KooWKUiRKKzspUQShSLWVwxgp1HnSAs3EgDLCQbXn5iGHGhF/p2p-circuit/p2p/16Uiu2HAm5HuMtmkgC6yPqq6g8NSrgqjbamQ6Vj6r3GjjrzKAs2Eu"
            .parse()
            .unwrap();
        let dns4: Multiaddr = "/dns4/coord.ghalbol.com/tcp/4002/p2p/12D3KooWKUiRKKzspUQShSLWVwxgp1HnSAs3EgDLCQbXn5iGHGhF/p2p-circuit/p2p/16Uiu2HAm5HuMtmkgC6yPqq6g8NSrgqjbamQ6Vj6r3GjjrzKAs2Eu"
            .parse()
            .unwrap();
        let out = wan_coord_dial_addrs(vec![v6.clone(), v4.clone(), dns4.clone()]);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0], v4);
        assert_eq!(out[1], dns4);
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
    fn bootstrap_family_rank_ipv6_degraded_prefers_v4_on_lan() {
        let v4: Multiaddr =
            "/ip4/34.30.211.249/tcp/4002/p2p/12D3KooWKUiRKKzspUQShSLWVwxgp1HnSAs3EgDLCQbXn5iGHGhF"
                .parse()
                .unwrap();
        let v6: Multiaddr =
            "/ip6/2600:1900:4000:8dad::/tcp/4002/p2p/12D3KooWKUiRKKzspUQShSLWVwxgp1HnSAs3EgDLCQbXn5iGHGhF"
                .parse()
                .unwrap();
        assert!(
            relay_bootstrap_family_rank(&v4, false, false)
                > relay_bootstrap_family_rank(&v6, false, false)
        );
        assert!(
            relay_bootstrap_family_rank(&v4, false, true)
                < relay_bootstrap_family_rank(&v6, false, true)
        );
    }

    #[test]
    fn os_network_snapshot_json_for_ui() {
        let snap = OsNetworkSnapshot {
            default_transport: OsDefaultTransport::Wifi,
            internet_validated: true,
            has_internet: true,
            wifi_link_up: true,
            default_route_iface: Some("wlan0".into()),
        };
        let j = os_network_snapshot_to_json(&snap, "test");
        assert_eq!(j["ok"], true);
        assert_eq!(j["default_transport"], "wifi");
        assert_eq!(j["internet_validated"], true);
        assert_eq!(j["default_route_iface"], "wlan0");
        assert_eq!(j["source"], "test");
    }

    #[test]
    fn os_cellular_default_overrides_lingering_wifi_ifaces() {
        let mut p = LocalNetworkProfile {
            has_rfc1918_ipv4: true,
            has_rfc1918_on_wifi: true,
            has_wifi_iface: true,
            has_cellular_iface: true,
            ..Default::default()
        };
        p.os.default_transport = OsDefaultTransport::Cellular;
        p.os.wifi_link_up = false;
        assert!(!p.has_active_lan());
        assert!(p.on_mobile_data_path());
        assert_eq!(p.mode_label(), "mobile-data");
    }

    #[test]
    fn os_wifi_default_is_lan_even_before_if_addrs_catch_up() {
        let p = LocalNetworkProfile {
            os: OsNetworkSnapshot {
                default_transport: OsDefaultTransport::Wifi,
                internet_validated: true,
                has_internet: true,
                wifi_link_up: true,
                default_route_iface: Some("wlan0".to_string()),
            },
            ..Default::default()
        };
        assert!(p.has_active_lan());
        assert!(!p.on_mobile_data_path());
        assert_eq!(p.mode_label(), "lan");
    }

    #[test]
    fn handover_key_changes_when_os_default_transport_changes() {
        let mut wifi = LocalNetworkProfile::default();
        wifi.os.default_transport = OsDefaultTransport::Wifi;
        wifi.os.wifi_link_up = true;
        let mut cell = wifi.clone();
        cell.os.default_transport = OsDefaultTransport::Cellular;
        cell.os.wifi_link_up = false;
        assert_ne!(network_handover_key(&wifi), network_handover_key(&cell));
    }
}
