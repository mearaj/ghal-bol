//! Linux network truth — Wi‑Fi operstate + default IPv4 route iface.


use crate::p2p::network_transport::{OsDefaultTransport, OsNetworkSnapshot};


fn operstate_is_up(state: &str) -> bool {
    matches!(state.trim(), "up" | "unknown" | "dormant")
}

fn sysfs_has_wifi_iface() -> bool {
    let Ok(entries) = std::fs::read_dir("/sys/class/net") else {
        return false;
    };
    entries
        .flatten()
        .any(|e| e.file_name().to_string_lossy().starts_with("wl"))
}

fn read_any_wifi_oper_up() -> bool {
    if !sysfs_has_wifi_iface() {
        return true;
    }
    let Ok(entries) = std::fs::read_dir("/sys/class/net") else {
        return true;
    };
    for entry in entries.flatten() {
        if !entry.file_name().to_string_lossy().starts_with("wl") {
            continue;
        }
        let Ok(state) = std::fs::read_to_string(entry.path().join("operstate")) else {
            continue;
        };
        if operstate_is_up(&state) {
            return true;
        }
    }
    false
}

fn iface_oper_up(name: &str) -> bool {
    let path = format!("/sys/class/net/{name}/operstate");
    let Ok(state) = std::fs::read_to_string(path) else {
        return true;
    };
    operstate_is_up(&state)
}

fn iface_name_is_wifi(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.starts_with("wl") || n.contains("wlan") || n.contains("wifi")
}

fn iface_name_is_cellular(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.contains("wwan") || n.contains("rmnet") || n.contains("pdp")
}

/// Parse `/proc/net/route` for the default IPv4 route (destination `00000000`).
fn linux_default_ipv4_route_iface() -> Option<String> {
    let raw = std::fs::read_to_string("/proc/net/route").ok()?;
    let mut best: Option<(u32, String)> = None;
    for line in raw.lines().skip(1) {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 8 {
            continue;
        }
        if cols[1] != "00000000" {
            continue;
        }
        let metric = cols[6].parse::<u32>().unwrap_or(u32::MAX);
        let iface = cols[0].to_string();
        match &best {
            Some((m, _)) if metric >= *m => {}
            _ => best = Some((metric, iface)),
        }
    }
    best.map(|(_, iface)| iface)
}



/// Authoritative Linux connectivity snapshot for `LocalNetworkProfile`.
pub fn probe_connectivity_truth() -> OsNetworkSnapshot {
    let wifi_up = read_any_wifi_oper_up();
    let default_iface = linux_default_ipv4_route_iface();
    let default_transport = default_iface
        .as_deref()
        .map(classify_iface_transport)
        .unwrap_or(if wifi_up {
            OsDefaultTransport::Wifi
        } else {
            OsDefaultTransport::None
        });
    let route_oper_up = default_iface
        .as_deref()
        .map(iface_oper_up)
        .unwrap_or(true);
    OsNetworkSnapshot {
        default_transport,
        internet_validated: route_oper_up && default_transport != OsDefaultTransport::None,
        has_internet: route_oper_up,
        wifi_link_up: wifi_up,
        default_route_iface: default_iface,
    }
}

fn classify_iface_transport(name: &str) -> OsDefaultTransport {
    if iface_name_is_wifi(name) {
        OsDefaultTransport::Wifi
    } else if iface_name_is_cellular(name) {
        OsDefaultTransport::Cellular
    } else if name == "lo" {
        OsDefaultTransport::None
    } else {
        OsDefaultTransport::Ethernet
    }
}
