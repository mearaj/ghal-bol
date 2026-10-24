use super::util::chrono_now_ms;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};

static NETWORK_CHANGE_NOTIFY: AtomicBool = AtomicBool::new(false);
static RELAY_REFRESH_NOTIFY: AtomicBool = AtomicBool::new(false);
static DM_PRESENCE_WAKE_NOTIFY: AtomicBool = AtomicBool::new(false);
static LAST_PRESENCE_WAKE_NOTIFY_MS: AtomicI64 = AtomicI64::new(0);
static DROP_PENDING_CALL_INVITE: OnceLock<fn(&str)> = OnceLock::new();

pub fn set_drop_pending_call_invite_hook(f: fn(&str)) {
    let _ = DROP_PENDING_CALL_INVITE.set(f);
}

pub(crate) fn drop_pending_call_invite(call_id: &str) {
    if let Some(f) = DROP_PENDING_CALL_INVITE.get() {
        f(call_id);
    }
}

pub fn notify_network_change() {
    NETWORK_CHANGE_NOTIFY.store(true, Ordering::SeqCst);
}

pub fn notify_relay_refresh() {
    RELAY_REFRESH_NOTIFY.store(true, Ordering::SeqCst);
}

pub(crate) fn take_connect_upkeep_notify() -> bool {
    NETWORK_CHANGE_NOTIFY.swap(false, Ordering::SeqCst)
        | RELAY_REFRESH_NOTIFY.swap(false, Ordering::SeqCst)
        | DM_PRESENCE_WAKE_NOTIFY.swap(false, Ordering::SeqCst)
}

pub fn notify_dm_presence_wake() {
    let now = chrono_now_ms();
    let last = LAST_PRESENCE_WAKE_NOTIFY_MS.load(Ordering::Relaxed);
    if last > 0 && now.saturating_sub(last) < 5_000 {
        return;
    }
    LAST_PRESENCE_WAKE_NOTIFY_MS.store(now, Ordering::Relaxed);
    DM_PRESENCE_WAKE_NOTIFY.store(true, Ordering::SeqCst);
}
