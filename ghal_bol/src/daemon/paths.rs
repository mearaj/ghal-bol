//! Default runtime paths for the desktop P2P daemon.

use std::path::PathBuf;

/// `$XDG_RUNTIME_DIR/ghalbol/` (or `/tmp/ghalbol` — never touches `~/.local/share/com.ghalbol`).
pub fn runtime_ghalbol_dir() -> PathBuf {
    if let Ok(runtime) = std::env::var("XDG_RUNTIME_DIR") {
        if !runtime.is_empty() {
            return PathBuf::from(runtime).join("ghalbol");
        }
    }
    PathBuf::from("/tmp/ghalbol")
}

/// `$XDG_RUNTIME_DIR/ghalbol/p2p.sock` when set, else under the app data dir.
pub fn default_socket_path() -> PathBuf {
    runtime_ghalbol_dir().join("p2p.sock")
}

/// Touched by the daemon when the user clicks an incoming-call notification.
pub fn incoming_call_wake_path() -> PathBuf {
    runtime_ghalbol_dir().join("incoming_call_wake")
}

/// Signal the Flutter UI process to present and show the call screen.
pub fn touch_incoming_call_wake() {
    let dir = runtime_ghalbol_dir();
    let _ = std::fs::create_dir_all(&dir);
    let path = incoming_call_wake_path();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis().to_string())
        .unwrap_or_else(|_| "1".to_string());
    let _ = std::fs::write(path, ts);
}
