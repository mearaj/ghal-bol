//! Best-effort raise of the Flutter desktop shell on Linux (D-Bus + gtk-launch + wake file).

use std::process::Command;

use crate::daemon::{touch_incoming_call_wake, touch_unlock_wake};

pub fn dbus_object_path(app_id: &str) -> String {
    format!("/{}", app_id.replace('.', "/"))
}

/// Write [touch_wake], call `org.freedesktop.Application.Activate`, then `gtk-launch`.
pub fn wake_desktop_app(app_id: &str, touch_wake: fn()) {
    let app_id = app_id.trim();
    if app_id.is_empty() {
        return;
    }
    touch_wake();
    let object_path = dbus_object_path(app_id);
    let _ = Command::new("gdbus")
        .args([
            "call",
            "-e",
            "-d",
            app_id,
            "-o",
            &object_path,
            "-m",
            "org.freedesktop.Application.Activate",
            "{}",
        ])
        .status();
    let _ = Command::new("gtk-launch").arg(app_id).spawn();
}

pub fn wake_for_unlock(app_id: &str) {
    wake_desktop_app(app_id, touch_unlock_wake);
}

pub fn wake_for_incoming_call(app_id: &str) {
    wake_desktop_app(app_id, touch_incoming_call_wake);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dbus_object_path_from_app_id() {
        assert_eq!(
            dbus_object_path("com.ghalbol.debug"),
            "/com/ghalbol/debug"
        );
    }
}
