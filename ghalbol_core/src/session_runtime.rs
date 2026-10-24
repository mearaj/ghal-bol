//! In-process identity session for the desktop app.

use std::panic::{self, AssertUnwindSafe};
use std::sync::{Mutex, OnceLock};

use serde_json::Value;

use crate::app_paths::resolved_storage_config;
use crate::dm_event_handler::set_p2p_handler_context;

fn session_mx() -> &'static Mutex<Option<crate::DecryptedIdentity>> {
    static S: OnceLock<Mutex<Option<crate::DecryptedIdentity>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(None))
}

pub(crate) fn unlocked_identity_clone() -> Result<crate::DecryptedIdentity, &'static str> {
    session_mx()
        .lock()
        .map_err(|_| "session mutex poisoned")?
        .as_ref()
        .cloned()
        .ok_or("identity not unlocked")
}

fn install_session(ident: crate::DecryptedIdentity) -> Result<(), &'static str> {
    session_mx()
        .lock()
        .map_err(|_| "session mutex poisoned")
        .map(|mut g| {
            let _prev = g.replace(ident);
            drop(_prev);
        })
}

pub fn unlock_identity(app_namespace: &str, password: &str) -> Value {
    let cfg = resolved_storage_config(app_namespace);
    let unlocked = panic::catch_unwind(AssertUnwindSafe(|| {
        crate::create_or_unlock_identity_v1(&cfg, password)
    }));
    match unlocked {
        Ok(Ok(ident)) => {
            let pk = ident.public_key_hex();
            let libp2p_peer_id = Some(ident.identity_wire());
            if let Err(e) = install_session(ident) {
                return serde_json::json!({ "ok": false, "error": e });
            }
            set_p2p_handler_context(app_namespace);
            #[cfg(not(target_arch = "wasm32"))]
            crate::delivery_runtime::delivery_start();
            serde_json::json!({
                "ok": true,
                "app_namespace": app_namespace,
                "public_key_hex": pk,
                "libp2p_peer_id": libp2p_peer_id,
            })
        }
        Ok(Err(e)) => serde_json::json!({ "ok": false, "error": format!("{e}") }),
        Err(_) => serde_json::json!({ "ok": false, "error": "Rust panic during unlock" }),
    }
}

pub fn lock_identity() {
    if let Ok(mut g) = session_mx().lock() {
        *g = None;
    }
}

pub fn session_unlocked() -> bool {
    session_mx()
        .lock()
        .ok()
        .and_then(|g| g.as_ref().map(|_| ()))
        .is_some()
}

/// Installs the unlocked identity for the in-process session.
pub(crate) fn install_unlocked_identity(
    ident: crate::DecryptedIdentity,
) -> Result<(), &'static str> {
    install_session(ident)
}
