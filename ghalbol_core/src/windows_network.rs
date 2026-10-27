//! Windows network truth — adapter carrying the OS default IPv4 route.
//!
//! A connected UDP socket asks the routing table which local address reaches the internet
//! (no packet is sent). That address names the adapter; Windows adapter names ("Wi-Fi",
//! "Ethernet", "Cellular") classify the transport.

use std::net::{IpAddr, UdpSocket};

use crate::p2p::network_transport::{OsDefaultTransport, OsNetworkSnapshot};

/// Any public address works: only the route lookup matters.
const ROUTE_PROBE_ADDR: &str = "1.1.1.1:53";

fn iface_name_is_wifi(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.contains("wi-fi") || n.contains("wifi") || n.contains("wlan") || n.contains("wireless")
}

fn iface_name_is_cellular(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.contains("cellular") || n.contains("mobile broadband") || n.contains("wwan")
}

fn classify_iface_transport(name: &str) -> OsDefaultTransport {
    if iface_name_is_wifi(name) {
        OsDefaultTransport::Wifi
    } else if iface_name_is_cellular(name) {
        OsDefaultTransport::Cellular
    } else {
        OsDefaultTransport::Ethernet
    }
}

fn default_route_local_ip() -> Option<IpAddr> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect(ROUTE_PROBE_ADDR).ok()?;
    let ip = socket.local_addr().ok()?.ip();
    (!ip.is_unspecified()).then_some(ip)
}

/// Authoritative Windows connectivity snapshot for `LocalNetworkProfile`.
pub fn probe_connectivity_truth() -> OsNetworkSnapshot {
    let ifaces = if_addrs::get_if_addrs().unwrap_or_default();
    let wifi_up = ifaces
        .iter()
        .any(|i| !i.is_loopback() && i.ip().is_ipv4() && iface_name_is_wifi(&i.name));
    let default_iface = default_route_local_ip()
        .and_then(|ip| ifaces.iter().find(|i| i.ip() == ip).map(|i| i.name.clone()));
    let default_transport = default_iface
        .as_deref()
        .map(classify_iface_transport)
        .unwrap_or(OsDefaultTransport::None);
    let routed = default_transport != OsDefaultTransport::None;
    OsNetworkSnapshot {
        default_transport,
        internet_validated: routed,
        has_internet: routed,
        wifi_link_up: wifi_up,
        default_route_iface: default_iface,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adapter_names_classify() {
        assert_eq!(classify_iface_transport("Wi-Fi"), OsDefaultTransport::Wifi);
        assert_eq!(
            classify_iface_transport("Wi-Fi 2"),
            OsDefaultTransport::Wifi
        );
        assert_eq!(
            classify_iface_transport("Cellular"),
            OsDefaultTransport::Cellular
        );
        assert_eq!(
            classify_iface_transport("Ethernet"),
            OsDefaultTransport::Ethernet
        );
    }

    #[test]
    fn probe_reports_a_route_iface_only_with_a_transport() {
        let snap = probe_connectivity_truth();
        assert_eq!(
            snap.default_route_iface.is_some(),
            snap.default_transport != OsDefaultTransport::None
        );
    }
}
