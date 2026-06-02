//! FFI for non-secret app preferences (coord URL).

use std::ffi::{CStr, CString};
use std::os::raw::c_char;

use crate::c_ffi::resolved_storage_config;

fn json_ok(v: serde_json::Value) -> *mut c_char {
    CString::new(v.to_string())
        .unwrap_or_else(|_| CString::new(r#"{"ok":false,"error":"encode"}"#).unwrap())
        .into_raw()
}

fn json_err(msg: impl AsRef<str>) -> *mut c_char {
    json_ok(serde_json::json!({ "ok": false, "error": msg.as_ref() }))
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

/// `{ "ok": true, "base_url": "…"|null, "insecure_tls": bool }`
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_ffi_coord_settings_get(
    app_namespace_utf8: *const c_char,
) -> *mut c_char {
    let run = || -> *mut c_char {
        let ns = match unsafe { utf8(app_namespace_utf8, "app_namespace") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let cfg = resolved_storage_config(&ns);
        match crate::preferences_v1::coord_settings_get(&cfg) {
            Ok((url, tls)) => json_ok(serde_json::json!({
                "ok": true,
                "base_url": url,
                "insecure_tls": tls,
            })),
            Err(e) => json_err(format!("{e}")),
        }
    };
    run()
}
