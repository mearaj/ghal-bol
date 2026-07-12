//! Server instance identity for ops / migration verification.

/// Operator-visible instance id (`GHAL_BOL_DELIVERY_INSTANCE_ID` or hostname).
pub fn instance_id() -> String {
    std::env::var("GHAL_BOL_DELIVERY_INSTANCE_ID")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(read_hostname)
}

fn read_hostname() -> String {
    let host = std::fs::read_to_string("/etc/hostname")
        .or_else(|_| std::env::var("HOSTNAME"))
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    if host.is_empty() {
        "unknown".to_string()
    } else {
        host
    }
}
