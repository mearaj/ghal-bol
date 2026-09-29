//! Marker files so a notification tap can raise the running desktop app.

use std::path::PathBuf;

fn runtime_dir() -> PathBuf {
    if let Ok(runtime) = std::env::var("XDG_RUNTIME_DIR") {
        if !runtime.is_empty() {
            return PathBuf::from(runtime).join("ghal_bol");
        }
    }
    PathBuf::from("/tmp/ghal_bol")
}

fn marker(name: &str) -> PathBuf {
    runtime_dir().join(name)
}

fn touch(name: &str) {
    let dir = runtime_dir();
    let _ = std::fs::create_dir_all(&dir);
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis().to_string())
        .unwrap_or_else(|_| "1".to_string());
    let _ = std::fs::write(marker(name), ts);
}

fn take(name: &str) -> bool {
    match std::fs::remove_file(marker(name)) {
        Ok(()) => true,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(_) => false,
    }
}

pub fn touch_incoming_call_wake() {
    touch("incoming_call_wake");
}

pub fn take_incoming_call_wake() -> bool {
    take("incoming_call_wake")
}

pub fn clear_incoming_call_wake() {
    let _ = std::fs::remove_file(marker("incoming_call_wake"));
}

pub fn touch_unlock_wake() {
    touch("unlock_wake");
}

pub fn take_unlock_wake() -> bool {
    take("unlock_wake")
}
