//! Default runtime paths for the desktop P2P daemon.
//!
//! Multiple integrators on one machine must use distinct [`runtime_dir_for_app_namespace`]
//! values (via `GHAL_BOL_APP_NAMESPACE` or `GHAL_BOL_RUNTIME_DIR`). See `docs/DAEMON_INTEGRATOR.md`.

use std::path::PathBuf;

fn base_runtime_root() -> PathBuf {
    if let Ok(runtime) = std::env::var("XDG_RUNTIME_DIR") {
        if !runtime.is_empty() {
            return PathBuf::from(runtime).join("ghal_bol");
        }
    }
    PathBuf::from("/tmp/ghal_bol")
}

/// Safe single path segment from an integrator `app_namespace` (e.g. `com.example.app`).
pub fn sanitize_app_namespace_segment(app_namespace: &str) -> String {
    let trimmed = app_namespace.trim();
    if trimmed.is_empty() {
        return "default".to_string();
    }
    trimmed
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Runtime directory for one integrator: wake files, `ui_present`, default socket parent.
pub fn runtime_dir_for_app_namespace(app_namespace: &str) -> PathBuf {
    base_runtime_root().join(sanitize_app_namespace_segment(app_namespace))
}

/// Active runtime directory for this daemon process.
///
/// Priority: `GHAL_BOL_RUNTIME_DIR` → namespace-scoped dir from `GHAL_BOL_APP_NAMESPACE`
/// (defaults to segment `default` when unset).
pub fn runtime_ghal_bol_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("GHAL_BOL_RUNTIME_DIR") {
        let d = dir.trim();
        if !d.is_empty() {
            return PathBuf::from(d);
        }
    }
    let ns = std::env::var("GHAL_BOL_APP_NAMESPACE")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "default".to_string());
    runtime_dir_for_app_namespace(&ns)
}

/// Default Unix socket for the active runtime dir.
pub fn default_socket_path() -> PathBuf {
    runtime_ghal_bol_dir().join("p2p.sock")
}

/// Default Unix socket for a specific integrator namespace (SDK helper).
pub fn default_socket_path_for_app_namespace(app_namespace: &str) -> PathBuf {
    runtime_dir_for_app_namespace(app_namespace).join("p2p.sock")
}

/// Touched by the daemon when the user clicks an incoming-call notification.
pub fn incoming_call_wake_path() -> PathBuf {
    runtime_ghal_bol_dir().join("incoming_call_wake")
}

/// Signal the integrator UI to present and show the call screen.
pub fn touch_incoming_call_wake() {
    let dir = runtime_ghal_bol_dir();
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

/// Touched when the daemon needs the UI for keystore unlock (login after reboot).
pub fn unlock_wake_path() -> PathBuf {
    runtime_ghal_bol_dir().join("unlock_wake")
}

/// Signal the integrator UI to present the unlock screen (polled when the app is already running).
pub fn touch_unlock_wake() {
    let dir = runtime_ghal_bol_dir();
    let _ = std::fs::create_dir_all(&dir);
    let path = unlock_wake_path();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis().to_string())
        .unwrap_or_else(|_| "1".to_string());
    let _ = std::fs::write(path, ts);
}

/// True when the daemon wrote [unlock_wake_path] and the UI has not consumed it yet.
pub fn take_unlock_wake() -> bool {
    let path = unlock_wake_path();
    match std::fs::remove_file(&path) {
        Ok(()) => true,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(_) => false,
    }
}

/// Drop stale unlock wake marker (successful unlock or previous session).
pub fn clear_unlock_wake() {
    let _ = std::fs::remove_file(unlock_wake_path());
}

/// Touched by the integrator shell on startup so the daemon grace timer knows the UI is already open.
pub fn ui_presence_path() -> PathBuf {
    runtime_ghal_bol_dir().join("ui_present")
}

/// True when the integrator process has marked itself running (see `ui_present` in runtime dir).
pub fn ui_presence_active() -> bool {
    ui_presence_path().exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_app_namespace_replaces_unsafe_chars() {
        assert_eq!(
            sanitize_app_namespace_segment("com.example/my app"),
            "com.example_my_app"
        );
    }

    #[test]
    fn runtime_dir_for_namespace_isolated() {
        let a = runtime_dir_for_app_namespace("com.app.a");
        let b = runtime_dir_for_app_namespace("com.app.b");
        assert_ne!(a, b);
        assert!(a.ends_with("com.app.a"));
        assert!(b.ends_with("com.app.b"));
    }

    #[test]
    fn default_socket_path_for_namespace() {
        let p = default_socket_path_for_app_namespace("com.example.chat");
        assert_eq!(
            p,
            base_runtime_root()
                .join("com.example.chat")
                .join("p2p.sock")
        );
    }
}
