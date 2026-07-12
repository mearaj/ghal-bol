//! Background libp2p DM for **`dart:ffi`** (native targets only).

use std::ffi::{CStr, CString};
use std::os::raw::c_char;

use crate::p2p_runtime;

fn json_err(msg: impl AsRef<str>) -> *mut c_char {
    let v = serde_json::json!({ "ok": false, "error": msg.as_ref() });
    CString::new(v.to_string())
        .unwrap_or_else(|_| CString::new(r#"{"ok":false,"error":"ffi"}"#).unwrap())
        .into_raw()
}

fn json_ok(v: serde_json::Value) -> *mut c_char {
    CString::new(v.to_string())
        .unwrap_or_else(|_| CString::new(r#"{"ok":false,"error":"encode"}"#).unwrap())
        .into_raw()
}

unsafe fn utf8(c: *const c_char, ctx: &'static str) -> Result<String, String> {
    if c.is_null() {
        return Err(format!("null ({ctx})"));
    }
    unsafe { CStr::from_ptr(c) }
        .to_str()
        .map(ToOwned::to_owned)
        .map_err(|_| format!("utf-8 ({ctx})"))
}

/// Start libp2p DM in a background thread. JSON config:
/// `{ "bootstrap_peers": [], "dm_peers": [{ "public_key_hex": "<identity wire>" }] }`.
///
/// Requires an unlocked identity session from [`crate::ghal_bol_core_ffi_create_or_unlock_identity`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_core_ffi_p2p_start(config_json_utf8: *const c_char) -> *mut c_char {
    let run = || -> *mut c_char {
        let cfg_s = match unsafe { utf8(config_json_utf8, "p2p config") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let v: serde_json::Value = match serde_json::from_str(&cfg_s) {
            Ok(v) => v,
            Err(e) => return json_err(format!("p2p config json: {e}")),
        };
        json_ok(p2p_runtime::p2p_start(&v))
    };
    run()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_core_ffi_p2p_is_running() -> *mut c_char {
    json_ok(p2p_runtime::p2p_is_running())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_core_ffi_p2p_stop() {
    p2p_runtime::p2p_stop();
}

/// Send a signed, encrypted text DM to `recipient_public_key_hex` (valid identity wire).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_core_ffi_p2p_send_text_dm(
    recipient_public_key_hex_utf8: *const c_char,
    text_utf8: *const c_char,
) -> *mut c_char {
    let run = || -> *mut c_char {
        let recipient = match unsafe { utf8(recipient_public_key_hex_utf8, "recipient public key") }
        {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let text = match unsafe { utf8(text_utf8, "text") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        json_ok(p2p_runtime::p2p_send_text_dm(&recipient, &text))
    };
    run()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_core_ffi_p2p_requeue_outbound_dm(
    message_id_utf8: *const c_char,
    recipient_public_key_hex_utf8: *const c_char,
    text_utf8: *const c_char,
) -> *mut c_char {
    let run = || -> *mut c_char {
        let message_id = match unsafe { utf8(message_id_utf8, "message_id") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let recipient = match unsafe { utf8(recipient_public_key_hex_utf8, "recipient public key") }
        {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let text = match unsafe { utf8(text_utf8, "text") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        json_ok(p2p_runtime::p2p_requeue_outbound_dm(
            &message_id,
            &recipient,
            &text,
        ))
    };
    run()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_core_ffi_p2p_send_ack_dm(
    recipient_public_key_hex_utf8: *const c_char,
    ref_id_utf8: *const c_char,
    ack_kind_utf8: *const c_char,
) -> *mut c_char {
    let run = || -> *mut c_char {
        let recipient = match unsafe { utf8(recipient_public_key_hex_utf8, "recipient") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let ref_id = match unsafe { utf8(ref_id_utf8, "ref_id") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let kind_s = match unsafe { utf8(ack_kind_utf8, "ack_kind") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        json_ok(p2p_runtime::p2p_send_ack_dm(&recipient, &ref_id, &kind_s))
    };
    run()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_core_ffi_p2p_register_dm_peer(
    _peer_id_utf8: *const c_char,
    public_key_hex_utf8: *const c_char,
) -> *mut c_char {
    let run = || -> *mut c_char {
        let pk = match unsafe { utf8(public_key_hex_utf8, "public_key_hex") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        json_ok(p2p_runtime::p2p_register_dm_peer(&pk))
    };
    run()
}

/// When `enabled` is 0, native stops all `ack_read` (background / UI destroyed). Delivery acks continue.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_core_ffi_p2p_set_app_ack_read_enabled(enabled: u8) -> *mut c_char {
    json_ok(p2p_runtime::p2p_set_app_ack_read_enabled(enabled != 0))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_core_ffi_p2p_set_app_ui_visible(visible: u8) -> *mut c_char {
    json_ok(p2p_runtime::p2p_set_app_ui_visible(visible != 0))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_core_ffi_p2p_set_foreground_peer(
    public_key_hex_utf8: *const c_char,
) -> *mut c_char {
    let run = || -> *mut c_char {
        let pk = if public_key_hex_utf8.is_null() {
            None
        } else {
            match unsafe { utf8(public_key_hex_utf8, "public_key_hex") } {
                Ok(s) => {
                    let s = s.trim();
                    if s.is_empty() {
                        None
                    } else {
                        Some(s.to_string())
                    }
                }
                Err(e) => return json_err(e),
            }
        };
        json_ok(p2p_runtime::p2p_set_foreground_peer(pk.as_deref()))
    };
    run()
}

/// Voice-call signaling. JSON config — see `p2p_runtime::p2p_call_signal`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_core_ffi_p2p_call_signal(
    config_json_utf8: *const c_char,
) -> *mut c_char {
    let run = || -> *mut c_char {
        let cfg_s = match unsafe { utf8(config_json_utf8, "call config") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let v: serde_json::Value = match serde_json::from_str(&cfg_s) {
            Ok(v) => v,
            Err(e) => return json_err(format!("call config json: {e}")),
        };
        json_ok(p2p_runtime::p2p_call_signal(&v))
    };
    run()
}

/// Native voice-call media control. JSON config — see `p2p_runtime::p2p_call_media`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_core_ffi_p2p_call_media(
    config_json_utf8: *const c_char,
) -> *mut c_char {
    let run = || -> *mut c_char {
        let cfg_s = match unsafe { utf8(config_json_utf8, "call media config") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let v: serde_json::Value = match serde_json::from_str(&cfg_s) {
            Ok(v) => v,
            Err(e) => return json_err(format!("call media config json: {e}")),
        };
        json_ok(p2p_runtime::p2p_call_media(&v))
    };
    run()
}

/// Active call snapshot for UI re-sync. JSON config — see `p2p_runtime::p2p_call_status`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_core_ffi_p2p_call_status(
    config_json_utf8: *const c_char,
) -> *mut c_char {
    let run = || -> *mut c_char {
        let cfg_s = match unsafe { utf8(config_json_utf8, "call status config") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let v: serde_json::Value = match serde_json::from_str(&cfg_s) {
            Ok(v) => v,
            Err(_) => serde_json::json!({}),
        };
        json_ok(p2p_runtime::p2p_call_status(&v))
    };
    run()
}

/// Dismiss OS incoming-call alert in `:p2p`. JSON config ignored.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_core_ffi_p2p_dismiss_incoming_call_alert(
    _config_json_utf8: *const c_char,
) -> *mut c_char {
    json_ok(p2p_runtime::p2p_dismiss_incoming_call_alert())
}

/// Force-end active call (media + hangup). JSON: `{ "reason": "..." }`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_core_ffi_p2p_force_end_active_call(
    config_json_utf8: *const c_char,
) -> *mut c_char {
    let run = || -> *mut c_char {
        let cfg_s = match unsafe { utf8(config_json_utf8, "force end call config") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let v: serde_json::Value = match serde_json::from_str(&cfg_s) {
            Ok(v) => v,
            Err(_) => serde_json::json!({}),
        };
        let reason = v.get("reason").and_then(|x| x.as_str()).unwrap_or("ffi");
        json_ok(p2p_runtime::p2p_force_end_active_call(reason))
    };
    run()
}

/// Take incoming-call wake marker. JSON config ignored.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_core_ffi_p2p_take_incoming_call_wake(
    _config_json_utf8: *const c_char,
) -> *mut c_char {
    json_ok(p2p_runtime::p2p_take_incoming_call_wake())
}

/// Native video-call control. JSON config — see `p2p_runtime::p2p_call_video`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_core_ffi_p2p_call_video(
    config_json_utf8: *const c_char,
) -> *mut c_char {
    let run = || -> *mut c_char {
        let cfg_s = match unsafe { utf8(config_json_utf8, "call video config") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let v: serde_json::Value = match serde_json::from_str(&cfg_s) {
            Ok(v) => v,
            Err(e) => return json_err(format!("call video config json: {e}")),
        };
        json_ok(p2p_runtime::p2p_call_video(&v))
    };
    run()
}

/// Push one I420 camera frame from the UI (desktop capture). JSON config — see
/// `p2p_runtime::p2p_call_video_push_camera_frame`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_core_ffi_p2p_call_video_push_camera_frame(
    config_json_utf8: *const c_char,
) -> *mut c_char {
    let run = || -> *mut c_char {
        let cfg_s = match unsafe { utf8(config_json_utf8, "call video push frame config") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let v: serde_json::Value = match serde_json::from_str(&cfg_s) {
            Ok(v) => v,
            Err(e) => return json_err(format!("call video push frame config json: {e}")),
        };
        json_ok(p2p_runtime::p2p_call_video_push_camera_frame(&v))
    };
    run()
}

/// GPU texture shm metadata. JSON config — see `p2p_runtime::p2p_call_video_texture`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_core_ffi_p2p_call_video_texture(
    config_json_utf8: *const c_char,
) -> *mut c_char {
    let run = || -> *mut c_char {
        let cfg_s = match unsafe { utf8(config_json_utf8, "call video texture config") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let v: serde_json::Value = match serde_json::from_str(&cfg_s) {
            Ok(v) => v,
            Err(e) => return json_err(format!("call video texture config json: {e}")),
        };
        json_ok(p2p_runtime::p2p_call_video_texture(&v))
    };
    run()
}

/// Pull the latest decoded video frame (render path). JSON config — see
/// `p2p_runtime::p2p_call_video_frame`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_core_ffi_p2p_call_video_frame(
    config_json_utf8: *const c_char,
) -> *mut c_char {
    let run = || -> *mut c_char {
        let cfg_s = match unsafe { utf8(config_json_utf8, "call video frame config") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let v: serde_json::Value = match serde_json::from_str(&cfg_s) {
            Ok(v) => v,
            Err(e) => return json_err(format!("call video frame config json: {e}")),
        };
        json_ok(p2p_runtime::p2p_call_video_frame(&v))
    };
    run()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_core_ffi_p2p_poll_event() -> *mut c_char {
    match p2p_runtime::p2p_poll_event() {
        Some(j) => CString::new(j.to_string())
            .map(|c| c.into_raw())
            .unwrap_or(std::ptr::null_mut()),
        None => std::ptr::null_mut(),
    }
}
