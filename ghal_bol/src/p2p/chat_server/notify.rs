use super::prelude::*;
use super::util::{chrono_now_ms, PRESENCE_WAKE_NOTIFY_DEBOUNCE_MS};
static NETWORK_CHANGE_NOTIFY: AtomicBool = AtomicBool::new(false);
static RELAY_REFRESH_NOTIFY: AtomicBool = AtomicBool::new(false);
/// DM peer left LAN / link dropped — run coord lookup on the swarm loop without waiting for coord_tick.
static COORD_LOOKUP_NOTIFY: AtomicBool = AtomicBool::new(false);
/// Self re-registered on coord or app resumed — immediate rediscovery for known DM peers.
static DM_PRESENCE_WAKE_NOTIFY: AtomicBool = AtomicBool::new(false);
static LAST_PRESENCE_WAKE_NOTIFY_MS: AtomicI64 = AtomicI64::new(0);
/// Network handover or chat stream death — reopen `/ghal-bol/msg/1.0.0` without waiting for upkeep tick.
static STREAM_REOPEN_NOTIFY: AtomicBool = AtomicBool::new(false);
static DROP_PENDING_CALL_INVITE: OnceLock<fn(&str)> = OnceLock::new();

pub fn set_drop_pending_call_invite_hook(f: fn(&str)) {
    let _ = DROP_PENDING_CALL_INVITE.set(f);
}

pub(crate) fn drop_pending_call_invite(call_id: &str) {
    if let Some(f) = DROP_PENDING_CALL_INVITE.get() {
        f(call_id);
    }
}
/// Android `ConnectivityManager` reports default network has `TRANSPORT_WIFI`.
pub(crate) static ANDROID_WIFI_TRANSPORT: AtomicBool = AtomicBool::new(false);

/// Min interval between Wi‑Fi LAN recovery passes (ms).
pub(crate) const LAN_RECOVERY_MIN_MS: i64 = 5_000;

pub fn set_android_wifi_transport_available(available: bool) {
    ANDROID_WIFI_TRANSPORT.store(available, Ordering::Relaxed);
}

/// Android connectivity / default-network change — swarm loop re-runs handover recovery.
pub fn notify_network_change() {
    NETWORK_CHANGE_NOTIFY.store(true, Ordering::SeqCst);
}

/// Re-fetch `/v1/relay` and re-dial the co-located relay (e.g. `p2p_start` `already_running`
/// while bore relay came up after the swarm started).
pub fn notify_relay_refresh() {
    RELAY_REFRESH_NOTIFY.store(true, Ordering::SeqCst);
}

pub(crate) fn take_network_change_notify() -> bool {
    NETWORK_CHANGE_NOTIFY.swap(false, Ordering::SeqCst)
}

pub(crate) fn take_relay_refresh_notify() -> bool {
    RELAY_REFRESH_NOTIFY.swap(false, Ordering::SeqCst)
}

pub(crate) fn notify_coord_lookup() {
    COORD_LOOKUP_NOTIFY.store(true, Ordering::SeqCst);
}

pub(crate) fn take_coord_lookup_notify() -> bool {
    COORD_LOOKUP_NOTIFY.swap(false, Ordering::SeqCst)
}

/// Wake DM rediscovery: self re-registered on coord, app resumed, or peer may have come online.
pub fn notify_dm_presence_wake() {
    let now = chrono_now_ms();
    let last = LAST_PRESENCE_WAKE_NOTIFY_MS.load(Ordering::Relaxed);
    if last > 0 && now.saturating_sub(last) < PRESENCE_WAKE_NOTIFY_DEBOUNCE_MS {
        return;
    }
    LAST_PRESENCE_WAKE_NOTIFY_MS.store(now, Ordering::Relaxed);
    DM_PRESENCE_WAKE_NOTIFY.store(true, Ordering::SeqCst);
    notify_coord_lookup();
}

pub(crate) fn take_dm_presence_wake_notify() -> bool {
    DM_PRESENCE_WAKE_NOTIFY.swap(false, Ordering::SeqCst)
}

pub(crate) fn notify_stream_reopen() {
    STREAM_REOPEN_NOTIFY.store(true, Ordering::SeqCst);
}

pub(crate) fn take_stream_reopen_notify() -> bool {
    STREAM_REOPEN_NOTIFY.swap(false, Ordering::SeqCst)
}
