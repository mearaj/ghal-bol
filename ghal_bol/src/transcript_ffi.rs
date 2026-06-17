//! FFI for [`crate::dm_transcript_store`].

use std::ffi::{CStr, CString};
use std::os::raw::c_char;

use serde_json::Value;

use crate::dm_transcript_store::{
    StoredChatLine, append_if_new, patch_inbound_read_ack_sent_for_thread, patch_outgoing_delivery,
    resolve_transcript_path, save_thread, thread_view,
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

fn line_from_value(v: &Value) -> Result<StoredChatLine, String> {
    StoredChatLine::from_json(v).ok_or_else(|| "invalid line json".to_string())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_ffi_transcript_resolve_path(
    app_namespace_utf8: *const c_char,
) -> *mut c_char {
    let run = || -> *mut c_char {
        let ns = match unsafe { utf8(app_namespace_utf8, "app_namespace") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        match resolve_transcript_path(&ns) {
            Ok(p) => json_ok(serde_json::json!({
                "ok": true,
                "path": p.to_string_lossy(),
            })),
            Err(e) => json_err(format!("{e}")),
        }
    };
    run()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_ffi_transcript_load_merged(
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
        let keys: Vec<String> = q
            .get("conversation_keys")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|x| x.as_str())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let from_peer = q.get("match_inbound_from_peer_id").and_then(|v| v.as_str());
        match crate::dm_transcript_store::thread_view(&ns, &keys, from_peer) {
            Ok(view) => json_ok(serde_json::json!({
                "ok": true,
                "revision": view.revision,
                "lines": view.lines.iter().map(|l| l.to_json()).collect::<Vec<_>>(),
            })),
            Err(e) => json_err(format!("{e}")),
        }
    };
    run()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_ffi_transcript_save(
    app_namespace_utf8: *const c_char,
    save_json_utf8: *const c_char,
) -> *mut c_char {
    let run = || -> *mut c_char {
        let ns = match unsafe { utf8(app_namespace_utf8, "app_namespace") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let sj = match unsafe { utf8(save_json_utf8, "save") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let v: Value = match serde_json::from_str(&sj) {
            Ok(v) => v,
            Err(e) => return json_err(format!("save json: {e}")),
        };
        let conv = v
            .get("conversation_key")
            .and_then(|x| x.as_str())
            .unwrap_or("");
        let lines: Vec<StoredChatLine> = v
            .get("lines")
            .and_then(|x| x.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|i| StoredChatLine::from_json(i))
                    .collect()
            })
            .unwrap_or_default();
        match save_thread(&ns, conv, lines) {
            Ok(()) => json_ok(serde_json::json!({ "ok": true })),
            Err(e) => json_err(format!("{e}")),
        }
    };
    run()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_ffi_transcript_append_if_new(
    app_namespace_utf8: *const c_char,
    append_json_utf8: *const c_char,
) -> *mut c_char {
    let run = || -> *mut c_char {
        let ns = match unsafe { utf8(app_namespace_utf8, "app_namespace") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let aj = match unsafe { utf8(append_json_utf8, "append") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let v: Value = match serde_json::from_str(&aj) {
            Ok(v) => v,
            Err(e) => return json_err(format!("append json: {e}")),
        };
        let conv = v
            .get("conversation_key")
            .and_then(|x| x.as_str())
            .unwrap_or("");
        let Some(line_v) = v.get("line") else {
            return json_err("missing line");
        };
        let line = match line_from_value(line_v) {
            Ok(l) => l,
            Err(e) => return json_err(e),
        };
        match append_if_new(&ns, conv, line) {
            Ok(()) => json_ok(serde_json::json!({ "ok": true })),
            Err(e) => json_err(format!("{e}")),
        }
    };
    run()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_ffi_transcript_patch_outgoing_delivery(
    app_namespace_utf8: *const c_char,
    patch_json_utf8: *const c_char,
) -> *mut c_char {
    let run = || -> *mut c_char {
        let ns = match unsafe { utf8(app_namespace_utf8, "app_namespace") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let pj = match unsafe { utf8(patch_json_utf8, "patch") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let v: Value = match serde_json::from_str(&pj) {
            Ok(v) => v,
            Err(e) => return json_err(format!("patch json: {e}")),
        };
        let conv = v
            .get("conversation_key")
            .and_then(|x| x.as_str())
            .unwrap_or("");
        let mid = v.get("message_id").and_then(|x| x.as_str()).unwrap_or("");
        let delivery = v.get("delivery").and_then(|x| x.as_str()).unwrap_or("");
        match patch_outgoing_delivery(&ns, conv, mid, delivery) {
            Ok(changed) => json_ok(serde_json::json!({ "ok": true, "changed": changed })),
            Err(e) => json_err(format!("{e}")),
        }
    };
    run()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_ffi_transcript_patch_inbound_read_ack_sent(
    app_namespace_utf8: *const c_char,
    patch_json_utf8: *const c_char,
) -> *mut c_char {
    let run = || -> *mut c_char {
        let ns = match unsafe { utf8(app_namespace_utf8, "app_namespace") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let pj = match unsafe { utf8(patch_json_utf8, "patch") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let v: Value = match serde_json::from_str(&pj) {
            Ok(v) => v,
            Err(e) => return json_err(format!("patch json: {e}")),
        };
        let conv = v.get("conversation_key").and_then(|x| x.as_str());
        let mid = v.get("message_id").and_then(|x| x.as_str()).unwrap_or("");
        if let Some(conv) = conv {
            match patch_inbound_read_ack_sent_for_thread(&ns, conv, mid) {
                Ok(changed) => json_ok(serde_json::json!({ "ok": true, "changed": changed })),
                Err(e) => json_err(format!("{e}")),
            }
        } else {
            match crate::dm_transcript_store::patch_inbound_read_ack_sent_global(&ns, mid) {
                Ok(()) => json_ok(serde_json::json!({ "ok": true, "changed": true })),
                Err(e) => json_err(format!("{e}")),
            }
        }
    };
    run()
}
