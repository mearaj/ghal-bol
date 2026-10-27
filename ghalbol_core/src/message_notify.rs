//! New-message notifications: one per chat, replaced by each newer message and removed when
//! that chat is opened. Fired from `dm_event_handler` only when inbound mail bumps unread
//! (new, not a poll replay, and not read in the open room). The body names what arrived —
//! message text never leaves the app UI.

/// What arrived, for the notification body.
#[derive(Clone, Copy)]
pub(crate) enum Arrived {
    Text,
    Voice,
    File,
}

impl Arrived {
    fn body(self) -> &'static str {
        match self {
            Arrived::Text => "New message",
            Arrived::Voice => "Voice message",
            Arrived::File => "File",
        }
    }
}

/// `sender_pk` sent something the user has not seen yet.
pub(crate) fn notify_inbound(sender_pk: &str, arrived: Arrived) {
    let pk = sender_pk.trim();
    if pk.is_empty() {
        return;
    }
    let title = crate::host::peer_display_name(pk);
    imp::show(pk, &title, arrived.body());
}

/// The chat with `peer_pk` is open: its notification is no longer news.
pub(crate) fn clear_for_peer(peer_pk: &str) {
    let pk = peer_pk.trim();
    if !pk.is_empty() {
        imp::clear(pk);
    }
}

// Unit tests drive the inbound path: never pop real desktop banners from `cargo test`.
#[cfg(all(target_os = "linux", not(test)))]
mod imp {
    use std::collections::HashMap;
    use std::process::Command;
    use std::sync::{Mutex, OnceLock};

    use notify_rust::{Hint, Notification};

    /// Server notification id per chat, so a newer message replaces the banner.
    fn shown() -> &'static Mutex<HashMap<String, u32>> {
        static SHOWN: OnceLock<Mutex<HashMap<String, u32>>> = OnceLock::new();
        SHOWN.get_or_init(|| Mutex::new(HashMap::new()))
    }

    fn key(pk: &str) -> String {
        pk.to_ascii_lowercase()
    }

    pub fn show(pk: &str, title: &str, body: &str) {
        let pk = key(pk);
        let title = title.to_string();
        let body = body.to_string();
        // D-Bus round trips stay off the poll (UI) thread.
        std::thread::spawn(move || {
            let app_id = crate::host::default_app_namespace();
            let replace = shown().lock().ok().and_then(|g| g.get(&pk).copied());
            let mut n = Notification::new();
            n.appname(&app_id)
                .summary(&title)
                .body(&body)
                .hint(Hint::DesktopEntry(app_id.clone()))
                .action("default", "Open Ghal Bol");
            if let Some(id) = replace {
                n.id(id);
            }
            let Ok(handle) = n.show() else {
                return;
            };
            if replace.is_some() {
                // Same server id: the first banner's waiter still answers its click.
                return;
            }
            let id = handle.id();
            if let Ok(mut g) = shown().lock() {
                g.insert(pk.clone(), id);
            }
            handle.wait_for_action(move |action| {
                if action == "default" {
                    crate::linux_desktop_launch::wake_desktop_app(&app_id, || {});
                }
                if let Ok(mut g) = shown().lock() {
                    if g.get(&pk) == Some(&id) {
                        g.remove(&pk);
                    }
                }
            });
        });
    }

    pub fn clear(pk: &str) {
        let Some(id) = shown().lock().ok().and_then(|mut g| g.remove(&key(pk))) else {
            return;
        };
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
}

#[cfg(all(target_os = "android", not(test)))]
mod imp {
    pub fn show(pk: &str, title: &str, body: &str) {
        if let Err(e) = crate::incoming_call_android::post_message(pk, title, body) {
            crate::flow_log::warn("DM/notify", format!("android message notify failed: {e}"));
        }
    }

    pub fn clear(pk: &str) {
        crate::incoming_call_android::cancel_message(pk);
    }
}

#[cfg(any(test, not(any(target_os = "linux", target_os = "android"))))]
mod imp {
    pub fn show(_pk: &str, _title: &str, _body: &str) {}

    pub fn clear(_pk: &str) {}
}
