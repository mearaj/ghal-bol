//! Track live Flutter UI RPC sockets. When the last UI disconnects, tear down any active call.

use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static UI_SESSIONS: AtomicUsize = AtomicUsize::new(0);
static UI_EXIT_HANGUP_SUPPRESS_UNTIL_MS: AtomicI64 = AtomicI64::new(0);

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// While reconnecting UI sockets (login unlock), do not hang up on transient EOF.
pub fn suppress_ui_exit_hangup_ms(ms: i64) {
    let until = now_ms().saturating_add(ms.max(0));
    UI_EXIT_HANGUP_SUPPRESS_UNTIL_MS.store(until, Ordering::SeqCst);
}

/// Whether at least one Flutter UI RPC socket is connected to the daemon.
pub fn ui_session_active() -> bool {
    UI_SESSIONS.load(Ordering::SeqCst) > 0
}

/// Explicit UI exit (best-effort before process death).
pub fn ui_process_exiting() {
    let _ = crate::p2p_runtime::p2p_force_end_active_call("ui_process_exiting");
}

pub struct UiSessionGuard;

impl UiSessionGuard {
    pub fn begin() -> Self {
        UI_SESSIONS.fetch_add(1, Ordering::SeqCst);
        Self
    }
}

impl Drop for UiSessionGuard {
    fn drop(&mut self) {
        let prev = UI_SESSIONS.fetch_sub(1, Ordering::SeqCst);
        if prev != 1 {
            return;
        }
        crate::p2p::set_app_ui_visible(false);
        if now_ms() < UI_EXIT_HANGUP_SUPPRESS_UNTIL_MS.load(Ordering::SeqCst) {
            return;
        }
        let ended = crate::p2p_runtime::p2p_force_end_active_call("ui_session_ended");
        if ended
            .get("ended")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            eprintln!("ghal_bol_core_daemon: active call ended — last UI socket closed");
        }
    }
}
