//! FFI for [`crate::contacts_v1`].

use std::ffi::{CStr, CString};
use std::os::raw::c_char;

use serde_json::Value;

use crate::contacts_v1::{
    self, contacts_change_version, find_by_peer_id, find_by_public_key, list_contacts,
    merge_discovered_peer_id, record_inbound_preview, remove_contact, set_contact_trust,
    upsert_contact, SavedContact,
};

fn json_ok(v: Value) -> *mut c_char {
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

fn contact_from_value(v: &Value) -> Result<SavedContact, String> {
    SavedContact::from_json(v).ok_or_else(|| "invalid contact json".to_string())
}

fn contacts_to_json(list: Vec<SavedContact>) -> Value {
    Value::Array(list.iter().map(|c| c.to_json()).collect())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_ffi_contacts_change_version() -> *mut c_char {
    json_ok(serde_json::json!({
        "ok": true,
        "version": contacts_change_version(),
    }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_ffi_contacts_list(app_namespace_utf8: *const c_char) -> *mut c_char {
    let run = || -> *mut c_char {
        let ns = match unsafe { utf8(app_namespace_utf8, "app_namespace") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        match list_contacts(&ns) {
            Ok(list) => json_ok(serde_json::json!({
                "ok": true,
                "contacts": contacts_to_json(list),
                "version": contacts_change_version(),
            })),
            Err(e) => json_err(format!("{e}")),
        }
    };
    run()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_ffi_contacts_upsert(
    app_namespace_utf8: *const c_char,
    contact_json_utf8: *const c_char,
) -> *mut c_char {
    let run = || -> *mut c_char {
        let ns = match unsafe { utf8(app_namespace_utf8, "app_namespace") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let cj = match unsafe { utf8(contact_json_utf8, "contact") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let v: Value = match serde_json::from_str(&cj) {
            Ok(v) => v,
            Err(e) => return json_err(format!("contact json: {e}")),
        };
        let contact = match contact_from_value(&v) {
            Ok(c) => c,
            Err(e) => return json_err(e),
        };
        match upsert_contact(&ns, contact) {
            Ok(c) => json_ok(serde_json::json!({
                "ok": true,
                "contact": c.to_json(),
                "version": contacts_change_version(),
            })),
            Err(e) => json_err(format!("{e}")),
        }
    };
    run()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_ffi_contacts_remove(
    app_namespace_utf8: *const c_char,
    contact_json_utf8: *const c_char,
) -> *mut c_char {
    let run = || -> *mut c_char {
        let ns = match unsafe { utf8(app_namespace_utf8, "app_namespace") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let cj = match unsafe { utf8(contact_json_utf8, "contact") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let v: Value = match serde_json::from_str(&cj) {
            Ok(v) => v,
            Err(e) => return json_err(format!("contact json: {e}")),
        };
        let contact = match contact_from_value(&v) {
            Ok(c) => c,
            Err(e) => return json_err(e),
        };
        match remove_contact(&ns, &contact) {
            Ok(()) => json_ok(serde_json::json!({
                "ok": true,
                "version": contacts_change_version(),
            })),
            Err(e) => json_err(format!("{e}")),
        }
    };
    run()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_ffi_contacts_find(
    app_namespace_utf8: *const c_char,
    query_json_utf8: *const c_char,
) -> *mut c_char {
    let run = || -> *mut c_char {
        let ns = match unsafe { utf8(app_namespace_utf8, "app_namespace") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let qj = match unsafe { utf8(query_json_utf8, "query") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let q: Value = match serde_json::from_str(&qj) {
            Ok(v) => v,
            Err(e) => return json_err(format!("query json: {e}")),
        };
        let found = if let Some(pk) = q.get("public_key_hex").and_then(|v| v.as_str()) {
            find_by_public_key(&ns, pk).ok().flatten()
        } else if let Some(pid) = q.get("libp2p_peer_id").or_else(|| q.get("peer_id")).and_then(|v| v.as_str()) {
            find_by_peer_id(&ns, pid).ok().flatten()
        } else {
            None
        };
        json_ok(serde_json::json!({
            "ok": true,
            "contact": found.as_ref().map(|c| c.to_json()),
        }))
    };
    run()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_ffi_contacts_merge_discovered_peer_id(
    app_namespace_utf8: *const c_char,
    public_key_hex_utf8: *const c_char,
    libp2p_peer_id_utf8: *const c_char,
) -> *mut c_char {
    let run = || -> *mut c_char {
        let ns = match unsafe { utf8(app_namespace_utf8, "app_namespace") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let pk = match unsafe { utf8(public_key_hex_utf8, "public_key_hex") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let pid = match unsafe { utf8(libp2p_peer_id_utf8, "libp2p_peer_id") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        match merge_discovered_peer_id(&ns, &pk, &pid) {
            Ok(()) => json_ok(serde_json::json!({
                "ok": true,
                "version": contacts_change_version(),
            })),
            Err(e) => json_err(format!("{e}")),
        }
    };
    run()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_ffi_contacts_record_inbound_preview(
    app_namespace_utf8: *const c_char,
    preview_json_utf8: *const c_char,
) -> *mut c_char {
    let run = || -> *mut c_char {
        let ns = match unsafe { utf8(app_namespace_utf8, "app_namespace") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let pj = match unsafe { utf8(preview_json_utf8, "preview") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let v: Value = match serde_json::from_str(&pj) {
            Ok(v) => v,
            Err(e) => return json_err(format!("preview json: {e}")),
        };
        let pk = v
            .get("sender_public_key_hex")
            .or_else(|| v.get("public_key_hex"))
            .and_then(|x| x.as_str())
            .unwrap_or("");
        let preview = v.get("preview").and_then(|x| x.as_str()).unwrap_or("");
        let mark_unread = v.get("mark_unread").and_then(|x| x.as_bool()).unwrap_or(false);
        let at = v.get("message_at_ms").and_then(|x| x.as_i64());
        match record_inbound_preview(&ns, pk, preview, mark_unread, at) {
            Ok(()) => json_ok(serde_json::json!({
                "ok": true,
                "version": contacts_change_version(),
            })),
            Err(e) => json_err(format!("{e}")),
        }
    };
    run()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_ffi_contacts_set_trust(
    app_namespace_utf8: *const c_char,
    trust_json_utf8: *const c_char,
) -> *mut c_char {
    let run = || -> *mut c_char {
        let ns = match unsafe { utf8(app_namespace_utf8, "app_namespace") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let tj = match unsafe { utf8(trust_json_utf8, "trust") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let v: Value = match serde_json::from_str(&tj) {
            Ok(v) => v,
            Err(e) => return json_err(format!("trust json: {e}")),
        };
        let pk = v
            .get("public_key_hex")
            .and_then(|x| x.as_str())
            .unwrap_or("");
        let is_known = v.get("is_known").and_then(|x| x.as_bool());
        let is_blocked = v.get("is_blocked").and_then(|x| x.as_bool());
        if is_known.is_none() && is_blocked.is_none() {
            return json_err("is_known or is_blocked required");
        }
        match set_contact_trust(&ns, pk, is_known, is_blocked) {
            Ok(c) => json_ok(serde_json::json!({
                "ok": true,
                "contact": c.to_json(),
                "version": contacts_change_version(),
            })),
            Err(e) => json_err(format!("{e}")),
        }
    };
    run()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_ffi_contacts_clear_unread(
    app_namespace_utf8: *const c_char,
    public_key_hex_utf8: *const c_char,
) -> *mut c_char {
    let run = || -> *mut c_char {
        let ns = match unsafe { utf8(app_namespace_utf8, "app_namespace") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let pk = match unsafe { utf8(public_key_hex_utf8, "public_key_hex") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        match contacts_v1::clear_unread(&ns, &pk) {
            Ok(()) => json_ok(serde_json::json!({
                "ok": true,
                "version": contacts_change_version(),
            })),
            Err(e) => json_err(format!("{e}")),
        }
    };
    run()
}
