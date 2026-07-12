//! FFI: **`ghal_bol_connect_v1`** invite verification + offline auxiliary seal.

use std::ffi::{CStr, CString};
use std::os::raw::c_char;

use serde_json::Value;

use crate::c_ffi::ffi_unlocked_identity_clone;
use crate::connect_invite_v1::{
    build_connect_invite_wire_map, connect_invite_app_uri_from_wire_map,
    connect_invite_uri_from_wire_map, parse_connect_invite_uri,
    verify_ghal_bol_connect_invite_value,
};
use crate::offline_seal_v1::{open_sealed_secp256k1, seal_to_secp256k1_public};
use crate::public_key_util::{
    legacy_libp2p_peer_id_str_from_public_key_hex, legacy_public_key_from_peer_id_str,
    secp256k1_public_key_from_hex,
};

fn json_ok(v: serde_json::Value) -> *mut c_char {
    CString::new(v.to_string())
        .unwrap_or_else(|_| CString::new(r#"{"ok":false,"error":"ffi encode"}"#).unwrap())
        .into_raw()
}

fn json_err(msg: impl AsRef<str>) -> *mut c_char {
    let v = serde_json::json!({ "ok": false, "error": msg.as_ref() });
    CString::new(v.to_string())
        .unwrap_or_else(|_| CString::new(r#"{"ok":false,"error":"ffi"}"#).unwrap())
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

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_core_ffi_verify_ghal_bol_connect_invite(
    invite_json_utf8: *const c_char,
) -> *mut c_char {
    let run = || -> *mut c_char {
        let s = match unsafe { utf8(invite_json_utf8, "invite json") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let v: Value = match serde_json::from_str(&s) {
            Ok(v) => v,
            Err(e) => return json_err(format!("invite json: {e}")),
        };
        if let Err(e) = verify_ghal_bol_connect_invite_value(&v) {
            return json_err(e);
        }
        json_ok(serde_json::json!({ "ok": true }))
    };
    run()
}

/// Extract identity wire embedded in an inline libp2p identity PeerId (secp256k1 / ed25519 only).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_core_ffi_public_key_hex_from_peer_id(
    peer_id_utf8: *const c_char,
) -> *mut c_char {
    let run = || -> *mut c_char {
        let s = match unsafe { utf8(peer_id_utf8, "peer_id") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        match legacy_public_key_from_peer_id_str(&s) {
            Some(pk) => json_ok(serde_json::json!({ "ok": true, "public_key_hex": pk })),
            None => json_err("peer_id does not embed a secp256k1 identity key"),
        }
    };
    run()
}

/// Legacy libp2p PeerId string for a secp256k1 public key (transcript thread migration only).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_core_ffi_peer_id_from_public_key_hex(
    public_key_hex_utf8: *const c_char,
) -> *mut c_char {
    let run = || -> *mut c_char {
        let s = match unsafe { utf8(public_key_hex_utf8, "public_key_hex") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let pk = s.trim().to_ascii_lowercase();
        match secp256k1_public_key_from_hex(&pk) {
            Ok(_) => {
                let peer_id =
                    legacy_libp2p_peer_id_str_from_public_key_hex(&pk).unwrap_or_default();
                json_ok(serde_json::json!({
                    "ok": true,
                    "public_key_hex": pk,
                    "peer_id": peer_id,
                }))
            }
            Err(e) => json_err(e),
        }
    };
    run()
}

/// Seal UTF-8 plaintext to a recipient identity public key (legacy secp256k1 invite path).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_core_ffi_seal_utf8_to_public_key_hex(
    recipient_public_key_hex_utf8: *const c_char,
    plaintext_utf8: *const c_char,
) -> *mut c_char {
    let run = || -> *mut c_char {
        let pk_s = match unsafe { utf8(recipient_public_key_hex_utf8, "recipient pk") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let pk = match secp256k1_public_key_from_hex(&pk_s) {
            Ok(p) => p,
            Err(e) => return json_err(e),
        };
        let text = match unsafe { utf8(plaintext_utf8, "plaintext") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let sealed = match seal_to_secp256k1_public(&pk.to_bytes(), text.as_bytes()) {
            Ok(b) => b,
            Err(e) => return json_err(e),
        };
        json_ok(serde_json::json!({
            "ok": true,
            "cipher_hex": hex::encode(sealed),
        }))
    };
    run()
}

/// Build format-2 connect invite URI. JSON: `{ "topic", "public_key_hex", "peer_alias"? }`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_core_ffi_build_connect_invite_uri(
    params_json_utf8: *const c_char,
) -> *mut c_char {
    let run = || -> *mut c_char {
        let s = match unsafe { utf8(params_json_utf8, "params") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let v: Value = match serde_json::from_str(&s) {
            Ok(v) => v,
            Err(e) => return json_err(format!("params json: {e}")),
        };
        let topic = v
            .get("topic")
            .and_then(|x| x.as_str())
            .unwrap_or("ghal-bol-chat");
        let pk = v
            .get("public_key_hex")
            .and_then(|x| x.as_str())
            .unwrap_or("");
        let alias = v.get("peer_alias").and_then(|x| x.as_str());
        let wire = match build_connect_invite_wire_map(topic, pk, alias) {
            Ok(w) => w,
            Err(e) => return json_err(e),
        };
        let uri = match connect_invite_uri_from_wire_map(&wire) {
            Ok(u) => u,
            Err(e) => return json_err(e),
        };
        let app_uri = match connect_invite_app_uri_from_wire_map(&wire) {
            Ok(u) => u,
            Err(e) => return json_err(e),
        };
        json_ok(serde_json::json!({
            "ok": true,
            "uri": uri,
            "app_uri": app_uri,
            "wire": wire,
        }))
    };
    run()
}

/// Parse connect invite URI (`https://ghalbol.com/connect/…` or `ghalbol://connect/…`). Returns wire map (verify separately if needed).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_core_ffi_parse_connect_invite_uri(
    uri_utf8: *const c_char,
) -> *mut c_char {
    let run = || -> *mut c_char {
        let s = match unsafe { utf8(uri_utf8, "uri") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        match parse_connect_invite_uri(&s) {
            Ok(wire) => json_ok(serde_json::json!({ "ok": true, "wire": wire })),
            Err(e) => json_err(e),
        }
    };
    run()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_core_ffi_open_sealed_cipher_hex(
    cipher_hex_utf8: *const c_char,
) -> *mut c_char {
    let run = || -> *mut c_char {
        let hex_s = match unsafe { utf8(cipher_hex_utf8, "cipher_hex") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let bytes = match hex::decode(hex_s.trim()) {
            Ok(b) => b,
            Err(e) => return json_err(format!("cipher_hex: {e}")),
        };
        let id = match ffi_unlocked_identity_clone() {
            Ok(i) => i,
            Err(e) => return json_err(e),
        };
        let plain = match open_sealed_secp256k1(id.secp256k1_secret(), &bytes) {
            Ok(p) => p,
            Err(e) => return json_err(e),
        };
        let text = match String::from_utf8(plain) {
            Ok(s) => s,
            Err(e) => return json_err(format!("plaintext utf-8: {e}")),
        };
        json_ok(serde_json::json!({ "ok": true, "plaintext": text }))
    };
    run()
}
