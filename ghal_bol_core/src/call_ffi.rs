//! FFI: per-call media keys for Flutter WebRTC FrameCryptor (transport KEM).

use std::ffi::CStr;
use std::ffi::CString;
use std::os::raw::c_char;

use crate::c_ffi::ffi_unlocked_identity_clone;
use crate::call_media_key::derive_call_media_keys_from_transport;
use crate::connect::transport_kem_for_peer;

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

/// JSON in: `{ "call_id": "…", "peer_public_key_hex": "<identity wire>" }`
/// JSON out: `{ "ok": true, "key_hex", "ratchet_salt_hex", "local_identity_wire" }`
///
/// Keys use the **unlocked device identity** and peer identity wire + `call_id`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_core_ffi_call_media_key_hex(
    config_json_utf8: *const c_char,
) -> *mut c_char {
    let run = || -> *mut c_char {
        let cfg_s = match unsafe { utf8(config_json_utf8, "call media key config") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let v: serde_json::Value = match serde_json::from_str(&cfg_s) {
            Ok(v) => v,
            Err(e) => return json_err(format!("config json: {e}")),
        };
        let call_id = match v.get("call_id").and_then(|x| x.as_str()) {
            Some(s) => s,
            None => return json_err("missing call_id"),
        };
        let peer = match v.get("peer_public_key_hex").and_then(|x| x.as_str()) {
            Some(s) => s,
            None => return json_err("missing peer_public_key_hex"),
        };
        let ident = match ffi_unlocked_identity_clone() {
            Ok(i) => i,
            Err(e) => return json_err(e),
        };
        let local_wire = ident.identity_wire();
        let (local_sk, peer_transport_pk) = match transport_kem_for_peer(peer) {
            Some(v) => v,
            None => return json_err("transport kem not ready for peer"),
        };
        let keys = match derive_call_media_keys_from_transport(
            &local_sk,
            &peer_transport_pk,
            &local_wire,
            peer,
            call_id,
        ) {
            Ok(k) => k,
            Err(e) => return json_err(e),
        };
        json_ok(serde_json::json!({
            "ok": true,
            "key_hex": hex::encode(keys.frame_key),
            "ratchet_salt_hex": hex::encode(keys.ratchet_salt),
            "local_identity_wire": local_wire,
            "local_public_key_hex": ident.public_key_hex(),
        }))
    };
    run()
}
