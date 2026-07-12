//! Out-of-process native DM node for Linux desktop (survives Flutter UI exit).
//!
//! `cargo run -p ghal_bol_core --bin ghal_bol_core_daemon`

use std::env;
use std::process;

use ghal_bol_core::daemon::{probe_existing_daemon, run_daemon, socket_path_from_env_or_default};

fn main() {
    ghal_bol_core::rustls_init::ensure_rustls_crypto_provider();
    let socket_path = socket_path_from_env_or_default();
    if env::args().any(|a| a == "--probe") {
        let ok = probe_existing_daemon(&socket_path);
        process::exit(if ok { 0 } else { 1 });
    }
    if probe_existing_daemon(&socket_path) {
        eprintln!(
            "ghal_bol_core_daemon already running at {}",
            socket_path.display()
        );
        return;
    }
    spawn_unlock_reminder();
    if let Err(e) = run_daemon(&socket_path) {
        eprintln!("ghal_bol_core_daemon failed: {e}");
        process::exit(1);
    }
}

/// When a keystore exists on disk, wait briefly for the UI to unlock. If still locked and no UI
/// socket is connected, raise the desktop app for password entry (plus a notification fallback).
fn spawn_unlock_reminder() {
    #[cfg(target_os = "linux")]
    {
        use ghal_bol_core::detect_keystore_app_namespace;
        use ghal_bol_core::daemon::{ui_presence_active, ui_session_active};
        use ghal_bol_core::wake_for_unlock;
        use ghal_bol_core::session_unlocked;

        let Some(app_id) = detect_keystore_app_namespace() else {
            return;
        };

        std::thread::spawn(move || {
            let grace_secs = std::env::var("GHAL_BOL_UNLOCK_GRACE_SECS")
                .ok()
                .and_then(|s| s.trim().parse().ok())
                .filter(|&n| n > 0)
                .unwrap_or(10);
            // If the user opened the app any time during grace (even if they closed it again),
            // do not auto-wake at the end — one shot per daemon start only when they never engaged.
            let mut user_engaged_ui = false;
            for _ in 0..grace_secs {
                std::thread::sleep(std::time::Duration::from_secs(1));
                if session_unlocked() {
                    return;
                }
                if ui_session_active() || ui_presence_active() {
                    user_engaged_ui = true;
                }
            }
            if session_unlocked() {
                return;
            }
            if user_engaged_ui || ui_session_active() || ui_presence_active() {
                return;
            }
            eprintln!(
                "ghal_bol_core_daemon: unlock wake — opening UI for password (app_id={app_id}, grace_secs={grace_secs})"
            );
            wake_for_unlock(&app_id);
            post_unlock_notification(&app_id);
        });
    }
}

#[cfg(target_os = "linux")]
fn post_unlock_notification(app_id: &str) {
    let app_id_owned = app_id.to_string();
    let Ok(handle) = notify_rust::Notification::new()
        .appname(&app_id_owned)
        .summary("Ghal Bol")
        .body("Enter your password to start receiving messages")
        .hint(notify_rust::Hint::DesktopEntry(app_id_owned.clone()))
        .hint(notify_rust::Hint::Urgency(notify_rust::Urgency::Normal))
        .action("default", "Open Ghal Bol")
        .show()
    else {
        return;
    };
    let launch_id = app_id_owned.clone();
    handle.wait_for_action(move |action| {
        // Dismiss / close must not re-open the app — only an explicit "Open" tap.
        if action != "default" {
            return;
        }
        if ghal_bol_core::session_unlocked() {
            return;
        }
        ghal_bol_core::wake_for_unlock(&launch_id);
    });
}
