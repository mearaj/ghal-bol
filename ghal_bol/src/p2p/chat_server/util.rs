pub(crate) fn chrono_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub(crate) const PRESENCE_WAKE_NOTIFY_DEBOUNCE_MS: i64 = 3_000;
pub(crate) const PRESENCE_WAKE_RUN_DEBOUNCE_MS: i64 = 4_000;
pub(crate) const COORD_PEER_NOT_ON_COORD_LOG_MIN_MS: i64 = 60_000;
