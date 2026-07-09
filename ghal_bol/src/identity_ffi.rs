//! FFI: multi-algorithm identity parse / compare (`ghal_bol_ffi_identity_*`).

use std::ffi::{CStr, CString};
use std::os::raw::c_char;

use crate::identity::{same_contact_identity, Identity, IdentityAlgorithm};
use crate::keystore_v1::parse_secret_bytes_for_algorithm;
use crate::public_key_util::normalize_contact_identity_wire;

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

/// List algorithms available for first-time identity creation.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_ffi_identity_supported_algorithms() -> *mut c_char {
    let algorithms: Vec<serde_json::Value> = IdentityAlgorithm::creatable_algorithms()
        .iter()
        .map(|algo| {
            serde_json::json!({
                "id": algo.wire_id(),
                "default": *algo == IdentityAlgorithm::Secp256k1,
                "p2p_ready": algo.p2p_ready(),
                "description": algo.create_description(),
                "import_secret_hint": algo.import_secret_hint(),
            })
        })
        .collect();
    json_value(serde_json::json!({ "ok": true, "algorithms": algorithms }))
}

/// Validate import secret before keystore write: `{ "algorithm": "…", "secret_hex": "…" }`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_ffi_identity_validate_import_secret(
    config_json_utf8: *const c_char,
) -> *mut c_char {
    let run = || -> *mut c_char {
        let cfg_s = match unsafe { utf8(config_json_utf8, "validate import secret") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let v: serde_json::Value = match serde_json::from_str(&cfg_s) {
            Ok(v) => v,
            Err(e) => return json_err(format!("validate import secret json: {e}")),
        };
        let algo_s = v
            .get("algorithm")
            .and_then(|x| x.as_str())
            .unwrap_or(IdentityAlgorithm::Secp256k1.wire_id());
        let algorithm = match IdentityAlgorithm::from_wire_id(algo_s) {
            Ok(a) => a,
            Err(e) => return json_err(e),
        };
        if !IdentityAlgorithm::creatable_algorithms().contains(&algorithm) {
            return json_err(format!("identity algorithm not available for creation: {algo_s}"));
        }
        let secret = v
            .get("secret_hex")
            .and_then(|x| x.as_str())
            .unwrap_or("");
        parse_secret_bytes_for_algorithm(algorithm, secret).map_or_else(
            |e| json_err(e.to_string()),
            |_| json_value(serde_json::json!({ "ok": true })),
        )
    };
    run()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_ffi_identity_parse(wire_utf8: *const c_char) -> *mut c_char {
    let run = || -> *mut c_char {
        let s = match unsafe { utf8(wire_utf8, "identity wire") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let id = match Identity::parse(&s) {
            Ok(id) => id,
            Err(e) => return json_err(e),
        };
        json_value(serde_json::json!({
            "ok": true,
            "wire": id.to_wire(),
            "algorithm": id.algorithm.wire_id(),
            "public_key_hex": id.public_key_hex(),
        }))
    };
    run()
}

/// Compare two identity wires: `{ "a": "...", "b": "..." }` → `{ ok, same }`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_ffi_identity_same(config_json_utf8: *const c_char) -> *mut c_char {
    let run = || -> *mut c_char {
        let cfg_s = match unsafe { utf8(config_json_utf8, "identity same") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let v: serde_json::Value = match serde_json::from_str(&cfg_s) {
            Ok(v) => v,
            Err(e) => return json_err(format!("identity same json: {e}")),
        };
        let a = v.get("a").and_then(|x| x.as_str()).unwrap_or("");
        let b = v.get("b").and_then(|x| x.as_str()).unwrap_or("");
        json_value(serde_json::json!({
            "ok": true,
            "same": same_contact_identity(a, b),
        }))
    };
    run()
}

/// Normalize identity wire: `{ "wire": "<identity>" }` → `{ ok, wire }`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_ffi_identity_normalize(
    wire_utf8: *const c_char,
) -> *mut c_char {
    let run = || -> *mut c_char {
        let s = match unsafe { utf8(wire_utf8, "identity normalize") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        match normalize_contact_identity_wire(&s) {
            Ok(wire) => json_value(serde_json::json!({ "ok": true, "wire": wire })),
            Err(e) => json_err(e),
        }
    };
    run()
}

#[cfg(test)]
mod identity_ffi_tests {
    use super::*;

    #[test]
    fn supported_algorithms_are_secp_ed25519_ecdsa_p256() {
        let raw = unsafe {
            let ptr = ghal_bol_ffi_identity_supported_algorithms();
            let s = std::ffi::CStr::from_ptr(ptr).to_string_lossy().into_owned();
            let _ = CString::from_raw(ptr);
            s
        };
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(v["ok"], true);
        let algos = v["algorithms"].as_array().unwrap();
        let ids: Vec<_> = algos
            .iter()
            .filter_map(|a| a.get("id").and_then(|x| x.as_str()))
            .collect();
        assert_eq!(ids, vec!["secp256k1", "ed25519", "ecdsa-p256"]);
        assert!(!ids.contains(&"ml-dsa-65"));
    }

    #[test]
    fn validate_import_secret_rejects_ml_dsa65() {
        let cfg = serde_json::json!({
            "algorithm": "ml-dsa-65",
            "secret_hex": "00".repeat(64),
        });
        let cfg_s = CString::new(cfg.to_string()).unwrap();
        let raw = unsafe {
            let ptr = ghal_bol_ffi_identity_validate_import_secret(cfg_s.as_ptr());
            let s = std::ffi::CStr::from_ptr(ptr).to_string_lossy().into_owned();
            let _ = CString::from_raw(ptr);
            s
        };
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_ne!(v["ok"], true);
    }
}
