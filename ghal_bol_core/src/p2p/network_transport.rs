//! Network profile detection, relay/coord dial helpers, and listen-address utilities (no Kademlia).

use std::collections::HashSet;

use crate::multiaddr_local::Multiaddr;

use super::native_log;

/// Session peer id — normalized identity wire (legacy name in dial helpers).
pub type PeerId = String;

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
/// Linux-only in-process FFI fallback; other platforms use the daemon `network_snapshot` RPC.
#[cfg(target_os = "linux")]
pub(crate) fn probe_os_network_truth_ui() -> Option<OsNetworkSnapshot> {
    Some(crate::linux_network::probe_connectivity_truth())
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


}

/// Android Wi‑Fi return can lag `if_addrs` while libp2p is already listening on RFC1918 TCP.
/// Only promote to LAN when the OS also reports Wi‑Fi linked — not from listen addr alone.

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


/// Append `/p2p-circuit` to the connected bootstrap TCP multiaddr (preserves live port).

/// Lower rank = preferred bootstrap TCP family for this profile.

/// CGNAT / shared address space (100.64.0.0/10) — Tailscale, carrier NAT, etc. Not home LAN RFC1918.

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


/// Inbound coord lookup addrs to dial when coord is primary (WAN paths only).
/// WAN = `/p2p-circuit` only — bare public TCP from coord is often relay bootstrap, not the peer.

/// Sort order for DM dials: LAN TCP first, then relay, then public WAN.




/// LAN-first when [peer_on_local_lan] (mDNS); WAN-first (relay, then public) otherwise.

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
    let s = ma.to_string();
    s.rsplit("/p2p/")
        .next()
        .and_then(|tail| tail.split('/').next())
        .map(|id| id.to_string())
        .filter(|id| !id.is_empty())
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


/// docker0 / podman / common CNI bridges — not reachable for DM dial (see user logs: 172.17–172.20).
pub(crate) fn is_docker_or_link_local_ipv4(ip: std::net::Ipv4Addr) -> bool {
    let o = ip.octets();
    (o[0] == 169 && o[1] == 254) || (o[0] == 172 && (17..=20).contains(&o[1]))
}

/// Dial only well-known bootstrap hosts at their resolved public IPs (or relay circuit).

/// Expand `0.0.0.0` / `::` listeners into concrete LAN addresses for coord/mDNS peerstore.


#[cfg(test)]
mod tests {
    use super::*;
    use crate::multiaddr_local::Multiaddr;

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

}
