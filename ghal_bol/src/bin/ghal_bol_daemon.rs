//! Out-of-process native DM node for Linux desktop (survives Flutter UI exit).
//!
//! `cargo run -p ghal_bol --bin ghal_bol_daemon`

use std::env;
use std::process;

use ghal_bol::daemon::{probe_existing_daemon, run_daemon, socket_path_from_env_or_default};

fn main() {
    let socket_path = socket_path_from_env_or_default();
    if env::args().any(|a| a == "--probe") {
        let ok = probe_existing_daemon(&socket_path);
        process::exit(if ok { 0 } else { 1 });
    }
    if probe_existing_daemon(&socket_path) {
        eprintln!(
            "ghal_bol_daemon already running at {}",
            socket_path.display()
        );
        return;
    }
    spawn_unlock_reminder();
    if let Err(e) = run_daemon(&socket_path) {
        eprintln!("ghal_bol_daemon failed: {e}");
        process::exit(1);
    }
}

/// If a keystore exists on disk, wait 10 s for the UI to unlock the session.
/// If still locked, post a desktop notification so the user knows to open the app.
fn spawn_unlock_reminder() {
    #[cfg(target_os = "linux")]
    {
        use ghal_bol::{StorageConfig, keystore_v1_file_exists, session_unlocked};

        let prod = keystore_v1_file_exists(&StorageConfig::new("com.ghalbol")).unwrap_or(false);
        let debug = keystore_v1_file_exists(&StorageConfig::new("com.ghalbol.debug")).unwrap_or(false);
        if !prod && !debug {
            return;
        }
        let app_id = if prod { "com.ghalbol" } else { "com.ghalbol.debug" };

        let app_id_owned = app_id.to_string();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(10));
            if session_unlocked() {
                return;
            }
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
            handle.wait_for_action(move |_action| {
                let _ = std::process::Command::new("gtk-launch")
                    .arg(&launch_id)
                    .spawn();
            });
        });
    }
}
