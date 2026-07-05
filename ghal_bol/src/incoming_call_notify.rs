//! Desktop incoming-call notification (Linux). Fired from `:p2p` / `ghal_bol_daemon` when an
//! invite arrives so the user can click to focus the app.

#[cfg(target_os = "linux")]
mod linux {
    use std::process::Command;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Mutex, RwLock};
    use std::thread;

    use notify_rust::{Hint, Notification, Timeout, Urgency};

    static ACTIVE: Mutex<Option<(String, u32)>> = Mutex::new(None);
    /// GTK `application-id` — `com.ghalbol.debug` for `flutter run`, `com.ghalbol` for release.
    static DESKTOP_APP_ID: RwLock<String> = RwLock::new(String::new());
    /// Set while we programmatically close a notification (hangup/dismiss) — not a user tap.
    static DISMISSING: AtomicBool = AtomicBool::new(false);

    fn short_pk(pk: &str) -> String {
        let p = pk.trim();
        if p.len() >= 16 {
            format!("{}…", &p[..8])
        } else if p.is_empty() {
            "Contact".to_string()
        } else {
            p.to_string()
        }
    }

    fn close_notification_by_id(id: u32) {
        DISMISSING.store(true, Ordering::Release);
        let _ = Command::new("dbus-send")
            .args([
                "--session",
                "--dest=org.freedesktop.Notifications",
                "--type=method_call",
                "/org/freedesktop/Notifications",
                "org.freedesktop.Notifications.CloseNotification",
                &format!("uint32:{id}"),
            ])
            .status();
    }

    fn desktop_app_id() -> String {
        DESKTOP_APP_ID
            .read()
            .ok()
            .and_then(|g| {
                let s = g.trim();
                if s.is_empty() {
                    None
                } else {
                    Some(s.to_string())
                }
            })
            .unwrap_or_else(|| crate::storage::ANDROID_LIBRARY_NAMESPACE.to_string())
    }

    /// Wake the Flutter UI: runtime wake file (polled by UI) + D-Bus activate + desktop entry.
    fn wake_ui() {
        crate::linux_desktop_launch::wake_for_incoming_call(&desktop_app_id());
    }

    pub fn show(peer_public_key_hex: &str, call_id: &str) {
        let call_id = call_id.trim();
        if call_id.is_empty() {
            return;
        }
        if let Ok(g) = ACTIVE.lock() {
            if g.as_ref().is_some_and(|(id, _)| id == call_id) {
                return;
            }
        }
        dismiss();
        let body = format!(
            "{} is calling — tap to answer in Ghal Bol",
            short_pk(peer_public_key_hex)
        );
        let app_id = desktop_app_id();
        let Ok(handle) = Notification::new()
            .appname(&app_id)
            .summary("Incoming call")
            .body(&body)
            .hint(Hint::DesktopEntry(app_id.into()))
            .hint(Hint::Urgency(Urgency::Critical))
            .timeout(Timeout::Milliseconds(
                crate::call_state::MAX_LIVE_CALL_INVITE_AGE_MS as u32,
            ))
            .action("default", "Open Ghal Bol")
            .show()
        else {
            return;
        };
        let id = handle.id();
        let ring_id = call_id.to_string();
        if let Ok(mut g) = ACTIVE.lock() {
            *g = Some((ring_id.clone(), id));
        }
        let ring_id_auto = ring_id.clone();
        thread::spawn(move || {
            let auto_dismiss_ms = crate::call_state::MAX_LIVE_CALL_INVITE_AGE_MS as u64;
            thread::sleep(std::time::Duration::from_millis(auto_dismiss_ms));
            let _ = ACTIVE.lock().map(|mut g| {
                if g.as_ref().is_some_and(|(cid, _)| cid == &ring_id_auto) {
                    if let Some((_, id)) = g.take() {
                        close_notification_by_id(id);
                    }
                }
            });
        });
        thread::spawn(move || {
            handle.wait_for_action(move |action| {
                let programmatic = DISMISSING.swap(false, Ordering::AcqRel);
                // Most DEs dismiss the banner on click → "__closed", not a custom action.
                if programmatic && action == "__closed" {
                    let _ = ACTIVE.lock().map(|mut g| {
                        if g.as_ref().is_some_and(|(cid, _)| cid == &ring_id) {
                            g.take();
                        }
                    });
                    return;
                }
                wake_ui();
                let _ = ACTIVE.lock().map(|mut g| {
                    if g.as_ref().is_some_and(|(cid, _)| cid == &ring_id) {
                        g.take();
                    }
                });
            });
        });
    }

    pub fn dismiss() {
        if let Ok(mut g) = ACTIVE.lock() {
            if let Some((_call_id, id)) = g.take() {
                close_notification_by_id(id);
            }
        }
    }

    pub fn set_desktop_app_id(app_namespace: &str) {
        let ns = app_namespace.trim();
        if ns.is_empty() {
            return;
        }
        if let Ok(mut g) = DESKTOP_APP_ID.write() {
            *g = ns.to_string();
        }
    }
}

#[cfg(target_os = "linux")]
pub fn set_desktop_app_id(app_namespace: &str) {
    linux::set_desktop_app_id(app_namespace);
}

#[cfg(not(target_os = "linux"))]
pub fn set_desktop_app_id(_app_namespace: &str) {}

#[cfg(target_os = "linux")]
pub fn show_incoming_call(peer_public_key_hex: &str, call_id: &str) {
    linux::show(peer_public_key_hex, call_id);
}

#[cfg(target_os = "linux")]
pub fn dismiss_incoming_call() {
    linux::dismiss();
}

#[cfg(target_os = "android")]
pub fn show_incoming_call(peer_public_key_hex: &str, call_id: &str) {
    crate::incoming_call_android::show(peer_public_key_hex, call_id);
}

#[cfg(target_os = "android")]
pub fn dismiss_incoming_call() {
    crate::incoming_call_android::dismiss();
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
pub fn show_incoming_call(_peer_public_key_hex: &str, _call_id: &str) {}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
pub fn dismiss_incoming_call() {}
