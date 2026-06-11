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

/// True when the daemon wrote [incoming_call_wake_path] and the UI has not consumed it yet.
pub fn take_incoming_call_wake() -> bool {
    let path = incoming_call_wake_path();
    match std::fs::remove_file(&path) {
        Ok(()) => true,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(_) => false,
    }
}

/// Drop stale wake marker (previous session notification tap).
pub fn clear_incoming_call_wake() {
    let _ = std::fs::remove_file(incoming_call_wake_path());
}
