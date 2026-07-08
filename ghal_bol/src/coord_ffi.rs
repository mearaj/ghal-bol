//! Coordination server FFI (`ghal_bol_ffi_coord_*`).

use std::ffi::{CStr, CString};
use std::os::raw::c_char;

use crate::coord_runtime;

fn json_err(msg: impl AsRef<str>) -> *mut c_char {
    let v = serde_json::json!({ "ok": false, "error": msg.as_ref() });
    CString::new(v.to_string())
        .unwrap_or_else(|_| CString::new(r#"{"ok":false,"error":"ffi"}"#).unwrap())
        .into_raw()
}

fn json_value(v: serde_json::Value) -> *mut c_char {
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

/// JSON config: `{ "base_urls": ["https://coord.example.com"], "insecure_tls": false }`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_ffi_coord_set_base_url(
    config_json_utf8: *const c_char,
) -> *mut c_char {
    let run = || -> *mut c_char {
        let cfg_s = match unsafe { utf8(config_json_utf8, "coord config") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let v: serde_json::Value = match serde_json::from_str(&cfg_s) {
            Ok(v) => v,
            Err(e) => return json_err(format!("coord config json: {e}")),
        };
        let insecure = v
            .get("insecure_tls")
            .and_then(|x| x.as_bool())
            .unwrap_or(false);
        let urls = coord_runtime::coord_urls_from_json_value(&v);
        if urls.is_empty() {
            return json_err("base_url or base_urls required");
        }
        json_value(coord_runtime::coord_set_base_urls_json(&urls, insecure))
    };
    run()
}

/// Lookup peer endpoints: `{ "public_key_hex": "<identity wire>" }` (bare hex = implicit secp256k1).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_ffi_coord_lookup_peer(
    config_json_utf8: *const c_char,
) -> *mut c_char {
    let run = || -> *mut c_char {
        let cfg_s = match unsafe { utf8(config_json_utf8, "coord lookup") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let v: serde_json::Value = match serde_json::from_str(&cfg_s) {
            Ok(v) => v,
            Err(e) => return json_err(format!("coord lookup json: {e}")),
        };
        let pk = v
            .get("public_key_hex")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .trim();
        if pk.is_empty() {
            return json_err("public_key_hex required");
        }
        let pk = match crate::public_key_util::normalize_contact_identity_wire(pk) {
            Ok(w) => w,
            Err(e) => return json_err(e),
        };
        json_value(coord_runtime::coord_lookup_peer_json(&pk))
    };
    run()
}

/// Force register with collected listen endpoints (after unlock + coord URL set).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_ffi_coord_register_now() -> *mut c_char {
    json_value(coord_runtime::coord_register_now_json())
}
