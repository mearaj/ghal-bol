use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) const COORD_PEER_NOT_ON_COORD_LOG_MIN_MS: i64 = 30_000;
pub(crate) const PRESENCE_WAKE_RUN_DEBOUNCE_MS: i64 = 2_000;
pub(crate) const PRESENCE_WAKE_NOTIFY_DEBOUNCE_MS: i64 = 5_000;

pub(crate) fn chrono_now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
