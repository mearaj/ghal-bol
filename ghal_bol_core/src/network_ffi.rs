//! OS network snapshot for Flutter UI — canonical implementation in `network_transport`.

use std::ffi::CString;
use std::os::raw::c_char;

use crate::p2p::network_transport::network_snapshot_for_ui;
#[cfg(target_os = "linux")]
use crate::p2p::network_transport::probe_os_network_truth_ui;

fn json_ok(v: serde_json::Value) -> *mut c_char {
    CString::new(v.to_string())
        .unwrap_or_else(|_| CString::new(r#"{"ok":false,"error":"encode"}"#).unwrap())
        .into_raw()
}

fn json_err(msg: impl AsRef<str>) -> *mut c_char {
    json_ok(serde_json::json!({ "ok": false, "error": msg.as_ref() }))
}

/// Daemon / `:p2p` JSON-RPC handler (`network_snapshot`).
pub fn network_snapshot_rpc() -> serde_json::Value {
    network_snapshot_for_ui("p2p")
}

/// In-process Linux UI fallback when daemon is not used.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_core_ffi_network_snapshot() -> *mut c_char {
    #[cfg(target_os = "linux")]
    {
        if let Some(snap) = probe_os_network_truth_ui() {
            return json_ok(crate::p2p::network_transport::os_network_snapshot_to_json(
                &snap, "ffi",
            ));
        }
    }
    json_err("use_daemon_network_snapshot")
}
