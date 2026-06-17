//! Linux Wi‑Fi link operstate — sysfs is faster/truer than if_addrs after toggle.

use std::sync::atomic::{AtomicBool, Ordering};

static WIFI_OPER_UP: AtomicBool = AtomicBool::new(true);

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

/// Current Wi‑Fi operstate (any `wl*` interface up). Non‑Wi‑Fi desktops always true.
pub fn wifi_oper_up() -> bool {
    WIFI_OPER_UP.load(Ordering::Relaxed)
}

/// Poll once per network tick; returns true on down→up transition (Wi‑Fi back).
pub fn poll_wifi_link_up_transition() -> bool {
    let up = read_any_wifi_oper_up();
    let prev = WIFI_OPER_UP.swap(up, Ordering::Relaxed);
    up && !prev
}
