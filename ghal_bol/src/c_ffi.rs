//! C ABI for **`dart:ffi`** and other native hosts (same crate as the keystore).
//!
//! Symbols: **`ghal_bol_ffi_string_free`**, **`ghal_bol_ffi_configure_android_data_directory`**,
//! **`ghal_bol_ffi_create_or_unlock_identity`**, **`ghal_bol_ffi_import_identity_from_secret_hex`**,
//! **`ghal_bol_ffi_reveal_secret_key_hex`**, **`ghal_bol_ffi_export_keystore_json`**,
//! **`ghal_bol_ffi_import_keystore_json`**, **`ghal_bol_ffi_reset_first_time_identity`**, **`ghal_bol_ffi_keystore_exists`**, **`ghal_bol_ffi_lock`**,
//! **`ghal_bol_ffi_delete_keystore`**,
//! **`ghal_bol_ffi_verify_ghal_bol_connect_invite`**, **`ghal_bol_ffi_peer_id_from_public_key_hex`**,
//! **`ghal_bol_ffi_public_key_hex_from_peer_id`**,
//! **`ghal_bol_ffi_seal_utf8_to_public_key_hex`**,
//! **`ghal_bol_ffi_peer_display_alias_get`**, **`ghal_bol_ffi_peer_display_alias_set`**.
//! (Names kept stable for existing embedders; the shared library is **`libghal_bol`**.)

#![allow(clippy::missing_safety_doc)]

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::panic::{self, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

fn android_data_dir_mx() -> &'static Mutex<Option<PathBuf>> {
    static D: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();
    D.get_or_init(|| Mutex::new(None))
}

/// Same paths as [`ghal_bol_ffi_configure_android_data_directory`] (P2P `:p2p` process).
#[cfg(any(target_os = "android", test))]
pub(crate) fn configure_android_data_directory(path: &str) {
    if let Ok(mut g) = android_data_dir_mx().lock() {
        *g = Some(PathBuf::from(path));
    }
}

/// Serialize tests that mutate the process-global Android/test data root.
#[cfg(test)]
pub(crate) fn test_storage_isolation_lock() -> &'static Mutex<()> {
    static L: OnceLock<Mutex<()>> = OnceLock::new();
    L.get_or_init(|| Mutex::new(()))
}

/// Android / `:p2p` data root when configured (shared with UI via `app_flutter`).
#[cfg(target_os = "android")]
pub(crate) fn optional_android_data_dir() -> Option<PathBuf> {
    android_data_dir_mx().lock().ok().and_then(|g| g.clone())
}

pub(crate) fn resolved_storage_config(ns: &str) -> crate::StorageConfig {
    let mut cfg = crate::StorageConfig::new(ns.to_owned());
    if let Ok(g) = android_data_dir_mx().lock() {
        if let Some(dir) = g.as_ref() {
            cfg = cfg.with_override_data_dir(dir.clone());
        }
    }
    cfg
}

pub(crate) fn ffi_unlocked_identity_clone() -> Result<crate::DecryptedIdentity, &'static str> {
    crate::session_runtime::unlocked_identity_clone()
}

unsafe fn utf8_trace(c: *const c_char, ctx: &'static str) -> Result<String, String> {
    if c.is_null() {
        return Err(format!("null pointer ({ctx})"));
    }
    unsafe { CStr::from_ptr(c) }
        .to_str()
        .map(ToOwned::to_owned)
        .map_err(|_| format!("invalid UTF-8 ({ctx})"))
}

fn json_ok(v: serde_json::Value) -> *mut c_char {
    CString::new(v.to_string())
        .unwrap_or_else(|_| {
            CString::new(r#"{"ok":false,"error":"ffi encoding ok payload"}"#).unwrap()
        })
        .into_raw()
}

fn json_err(msg: impl AsRef<str>) -> *mut c_char {
    let v = serde_json::json!({ "ok": false, "error": msg.as_ref() });
    CString::new(v.to_string())
        .unwrap_or_else(|_| CString::new(r#"{"ok":false,"error":"ffi encoding"}"#).unwrap())
        .into_raw()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_ffi_string_free(ptr: *mut c_char) {
    if ptr.is_null() {
        return;
    }
    drop(unsafe { CString::from_raw(ptr) });
}

/// Optional: point keystore persistence at **`Context.getFilesDir()`** (or equivalent) on Android.
/// Call before [`ghal_bol_ffi_create_or_unlock_identity`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_ffi_configure_android_data_directory(path_utf8: *const c_char) {
    let run = || -> Result<(), String> {
        let s = unsafe { utf8_trace(path_utf8, "android data dir") }?;
        let mut g = android_data_dir_mx()
            .lock()
            .map_err(|_| "android data dir mutex poisoned")?;
        *g = Some(PathBuf::from(s));
        Ok(())
    };
    let _ = run();
}

fn parse_optional_identity_algorithm(
    c: *const c_char,
) -> Result<Option<crate::IdentityAlgorithm>, String> {
    if c.is_null() {
        return Ok(None);
    }
    let s = unsafe { utf8_trace(c, "identity_algorithm") }?;
    if s.trim().is_empty() {
        return Ok(None);
    }
    crate::IdentityAlgorithm::from_wire_id(s.trim()).map(Some)
}

/// Create keystore + identity if missing, otherwise unlock existing. Password is UTF-8.
/// [identity_algorithm_utf8] is optional (null or empty → implicit secp256k1 on **create** only).
/// Returns UTF-8 JSON `{ ok, public_key_hex?, identity_wire?, identity_algorithm?, libp2p_peer_id?, p2p_ready?, error? }`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_ffi_create_or_unlock_identity(
    app_namespace_utf8: *const c_char,
    password_utf8: *const c_char,
    identity_algorithm_utf8: *const c_char,
) -> *mut c_char {
    let run = || -> *mut c_char {
        let ns = match unsafe { utf8_trace(app_namespace_utf8, "app_namespace") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let password = match unsafe { utf8_trace(password_utf8, "password") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let create_algorithm = match parse_optional_identity_algorithm(identity_algorithm_utf8)
        {
            Ok(v) => v,
            Err(e) => return json_err(e),
        };

        let cfg = resolved_storage_config(&ns);

        let unlocked = panic::catch_unwind(AssertUnwindSafe(|| {
            crate::create_or_unlock_identity_v1_with_algorithm(&cfg, &password, create_algorithm)
        }));
        match unlocked {
            Ok(Ok(ident)) => install_identity_session(&ns, ident),
            Ok(Err(e)) => json_err(format!("{e}")),
            Err(_) => json_err("Rust panic during unlock"),
        }
    };

    run()
}

fn install_identity_session(ns: &str, ident: crate::DecryptedIdentity) -> *mut c_char {
    let pk = ident.public_key_hex();
    let wire = ident.identity_wire();
    let algorithm = ident.algorithm().wire_id();
    let p2p_ready = ident.p2p_ready();
    let libp2p_peer_id = ident
        .to_libp2p_keypair()
        .ok()
        .map(|kp| kp.public().to_peer_id().to_string());
    if let Err(e) = crate::session_runtime::install_unlocked_identity(ident) {
        return json_err(e);
    }
    crate::dm_event_handler::set_p2p_handler_context(ns);
    json_ok(serde_json::json!({
        "ok": true,
        "app_namespace": ns,
        "public_key_hex": pk,
        "identity_wire": wire,
        "identity_algorithm": algorithm,
        "p2p_ready": p2p_ready,
        "libp2p_peer_id": libp2p_peer_id,
    }))
}

/// First-time import: secret hex + app password. Fails if keystore already exists.
/// [identity_algorithm_utf8] optional (null/empty → secp256k1).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_ffi_import_identity_from_secret_hex(
    app_namespace_utf8: *const c_char,
    password_utf8: *const c_char,
    secret_key_hex_utf8: *const c_char,
    identity_algorithm_utf8: *const c_char,
) -> *mut c_char {
    let run = || -> *mut c_char {
        let ns = match unsafe { utf8_trace(app_namespace_utf8, "app_namespace") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let password = match unsafe { utf8_trace(password_utf8, "password") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let secret_hex = match unsafe { utf8_trace(secret_key_hex_utf8, "secret_key_hex") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let algorithm = match parse_optional_identity_algorithm(identity_algorithm_utf8)
        {
            Ok(Some(a)) => a,
            Ok(None) => crate::IdentityAlgorithm::Secp256k1,
            Err(e) => return json_err(e),
        };
        let cfg = resolved_storage_config(&ns);
        let unlocked = panic::catch_unwind(AssertUnwindSafe(|| {
            crate::import_identity_from_secret_hex_v1_with_algorithm(
                &cfg,
                &password,
                &secret_hex,
                algorithm,
            )
        }));
        match unlocked {
            Ok(Ok(ident)) => install_identity_session(&ns, ident),
            Ok(Err(e)) => json_err(format!("{e}")),
            Err(_) => json_err("Rust panic during import"),
        }
    };
    run()
}

/// Returns UTF-8 JSON `{ ok, secret_key_hex? }` after verifying [password_utf8] against on-disk keystore.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_ffi_reveal_secret_key_hex(
    app_namespace_utf8: *const c_char,
    password_utf8: *const c_char,
) -> *mut c_char {
    let run = || -> *mut c_char {
        let ns = match unsafe { utf8_trace(app_namespace_utf8, "app_namespace") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let password = match unsafe { utf8_trace(password_utf8, "password") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let cfg = resolved_storage_config(&ns);
        match crate::reveal_secret_key_hex_v1(&cfg, &password) {
            Ok((hex, algo)) => json_ok(serde_json::json!({
                "ok": true,
                "secret_key_hex": hex,
                "identity_algorithm": algo.wire_id(),
            })),
            Err(e) => json_err(format!("{e}")),
        }
    };
    run()
}

/// Returns UTF-8 JSON `{ ok, keystore_json? }` (encrypted keystore file contents).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_ffi_export_keystore_json(
    app_namespace_utf8: *const c_char,
) -> *mut c_char {
    let run = || -> *mut c_char {
        let ns = match unsafe { utf8_trace(app_namespace_utf8, "app_namespace") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let cfg = resolved_storage_config(&ns);
        match crate::export_keystore_json_v1(&cfg) {
            Ok(json) => json_ok(serde_json::json!({
                "ok": true,
                "keystore_json": json,
            })),
            Err(e) => json_err(format!("{e}")),
        }
    };
    run()
}

/// Restore encrypted keystore JSON when none exists; [password_utf8] must unlock the blob.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_ffi_import_keystore_json(
    app_namespace_utf8: *const c_char,
    password_utf8: *const c_char,
    keystore_json_utf8: *const c_char,
) -> *mut c_char {
    let run = || -> *mut c_char {
        let ns = match unsafe { utf8_trace(app_namespace_utf8, "app_namespace") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let password = match unsafe { utf8_trace(password_utf8, "password") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let json = match unsafe { utf8_trace(keystore_json_utf8, "keystore_json") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let cfg = resolved_storage_config(&ns);
        let unlocked = panic::catch_unwind(AssertUnwindSafe(|| {
            crate::import_keystore_from_json_v1(&cfg, &password, &json)
        }));
        match unlocked {
            Ok(Ok(ident)) => install_identity_session(&ns, ident),
            Ok(Err(e)) => json_err(format!("{e}")),
            Err(_) => json_err("Rust panic during keystore import"),
        }
    };
    run()
}

/// Remove keystore after a failed first-time create/import so the user can retry with a new password.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_ffi_reset_first_time_identity(
    app_namespace_utf8: *const c_char,
) -> *mut c_char {
    let run = || -> *mut c_char {
        let ns = match unsafe { utf8_trace(app_namespace_utf8, "app_namespace") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let cfg = resolved_storage_config(&ns);
        crate::session_runtime::lock_identity();
        match crate::reset_first_time_identity_v1(&cfg) {
            Ok(()) => json_ok(serde_json::json!({ "ok": true })),
            Err(e) => json_err(format!("{e}")),
        }
    };
    run()
}

/// Returns UTF-8 JSON `{ "ok": true, "keystore_exists": <bool> }` for [app_namespace_utf8],
/// using the same storage paths as [`ghal_bol_ffi_create_or_unlock_identity`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_ffi_keystore_exists(
    app_namespace_utf8: *const c_char,
) -> *mut c_char {
    let run = || -> *mut c_char {
        let ns = match unsafe { utf8_trace(app_namespace_utf8, "app_namespace") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };

        let cfg = resolved_storage_config(&ns);

        match crate::keystore_v1_file_exists(&cfg) {
            Ok(exists) => json_ok(serde_json::json!({
                "ok": true,
                "keystore_exists": exists,
            })),
            Err(e) => json_err(format!("{e}")),
        }
    };

    run()
}

/// Unix P2P daemon socket path (`GHAL_BOL_DAEMON_SOCKET` or platform default).
#[cfg(all(not(target_arch = "wasm32"), unix))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_ffi_daemon_socket_path() -> *mut c_char {
    let p = crate::daemon::socket_path_from_env_or_default();
    json_ok(serde_json::json!({
        "ok": true,
        "path": p.to_string_lossy(),
    }))
}

/// Drop decrypted keys from RAM (session).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_ffi_lock() {
    crate::session_runtime::lock_identity();
}

/// Delete persisted keystore + preferences after verifying [password_utf8] unlocks the keystore.
/// Clears the in-memory session. Caller should stop P2P before calling (e.g. [`ghal_bol_ffi_p2p_stop`]).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_ffi_delete_keystore(
    app_namespace_utf8: *const c_char,
    password_utf8: *const c_char,
) -> *mut c_char {
    let run = || -> *mut c_char {
        let ns = match unsafe { utf8_trace(app_namespace_utf8, "app_namespace") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let password = match unsafe { utf8_trace(password_utf8, "password") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let cfg = resolved_storage_config(&ns);
        let stored = match crate::load_keystore_v1(&cfg) {
            Ok(Some(s)) => s,
            Ok(None) => return json_err("no keystore on disk"),
            Err(e) => return json_err(format!("{e}")),
        };
        if let Err(e) = crate::unlock_keystore_v1(&password, &stored.keystore) {
            return json_err(format!("{e}"));
        }
        crate::session_runtime::lock_identity();
        match crate::delete_stored_identity_v1(&cfg) {
            Ok(()) => json_ok(serde_json::json!({ "ok": true })),
            Err(e) => json_err(format!("{e}")),
        }
    };
    run()
}

/// Read UTF-8 JSON `{ "ok": true, "alias": <string|null> }` for the unlocked identity.
/// [public_key_hex_utf8] must match the session public key.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_ffi_peer_display_alias_get(
    app_namespace_utf8: *const c_char,
    public_key_hex_utf8: *const c_char,
) -> *mut c_char {
    let run = || -> *mut c_char {
        let ns = match unsafe { utf8_trace(app_namespace_utf8, "app_namespace") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let sig = match unsafe { utf8_trace(public_key_hex_utf8, "public_key_hex") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let ident = match ffi_unlocked_identity_clone() {
            Ok(i) => i,
            Err(e) => return json_err(e),
        };
        let cfg = resolved_storage_config(&ns);
        match crate::preferences_v1::peer_display_alias_get(&cfg, &ident, &sig) {
            Ok(opt) => json_ok(serde_json::json!({ "ok": true, "alias": opt })),
            Err(e) => json_err(format!("{e}")),
        }
    };
    run()
}

/// Write UTF-8 JSON `{ "ok": true, "alias": <string|null> }` after persisting.
/// Empty or whitespace [alias_utf8] clears the stored alias.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ghal_bol_ffi_peer_display_alias_set(
    app_namespace_utf8: *const c_char,
    public_key_hex_utf8: *const c_char,
    alias_utf8: *const c_char,
) -> *mut c_char {
    let run = || -> *mut c_char {
        let ns = match unsafe { utf8_trace(app_namespace_utf8, "app_namespace") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let sig = match unsafe { utf8_trace(public_key_hex_utf8, "public_key_hex") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let alias = match unsafe { utf8_trace(alias_utf8, "alias") } {
            Ok(s) => s,
            Err(e) => return json_err(e),
        };
        let ident = match ffi_unlocked_identity_clone() {
            Ok(i) => i,
            Err(e) => return json_err(e),
        };
        let cfg = resolved_storage_config(&ns);
        match crate::preferences_v1::peer_display_alias_set(&cfg, &ident, &sig, &alias) {
            Ok(opt) => json_ok(serde_json::json!({ "ok": true, "alias": opt })),
            Err(e) => json_err(format!("{e}")),
        }
    };
    run()
}
