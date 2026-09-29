//! Best-effort raise of the Makepad desktop shell on Linux (D-Bus + gtk-launch + wake file).

use std::process::Command;

use crate::desktop_wake::{touch_incoming_call_wake, touch_unlock_wake};

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

/// Login autostart and a menu entry for this binary. One process: the app itself.
pub fn install_user_autostart() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let Ok(home) = std::env::var("HOME") else {
        return;
    };
    let app_id = if cfg!(debug_assertions) {
        "com.ghalbol.debug"
    } else {
        "com.ghalbol"
    };
    let autostart = format!(
        "[Desktop Entry]\nType=Application\nName=Ghal Bol\nExec={exe}\nStartupNotify=true\nX-GNOME-Autostart-enabled=true\n",
        exe = exe.display()
    );
    let applications = format!(
        "[Desktop Entry]\nType=Application\nName=Ghal Bol\nExec={exe} %u\nStartupNotify=true\nMimeType=x-scheme-handler/ghalbol;\n",
        exe = exe.display()
    );
    let auto_dir = std::path::PathBuf::from(format!("{home}/.config/autostart"));
    let apps_dir = std::path::PathBuf::from(format!("{home}/.local/share/applications"));
    if std::fs::create_dir_all(&auto_dir).is_ok() {
        let _ = std::fs::write(auto_dir.join(format!("{app_id}.desktop")), autostart);
    }
    if std::fs::create_dir_all(&apps_dir).is_ok() {
        let file = apps_dir.join(format!("{app_id}.desktop"));
        let _ = std::fs::write(&file, applications);
        let _ = std::process::Command::new("xdg-mime")
            .args(["default", &format!("{app_id}.desktop"), "x-scheme-handler/ghalbol"])
            .status();
    }
}

pub fn notify_unlock_needed() {
    let _ = notify_rust::Notification::new()
        .summary("Ghal Bol")
        .body("Enter your app password to receive messages.")
        .timeout(notify_rust::Timeout::Milliseconds(8000))
        .show();
}

/// Short tone so an incoming call is audible while the window is open.
pub fn play_incoming_ring() {
    play_tone("ghal-bol-ring.wav", 880.0);
}

/// Short tone while an outgoing call is still connecting.
pub fn play_ringback() {
    play_tone("ghal-bol-ringback.wav", 440.0);
}

fn play_tone(file_name: &str, hz: f32) {
    let bytes = ring_tone(hz);
    let path = std::env::temp_dir().join(file_name);
    std::thread::spawn(move || {
        if std::fs::write(&path, bytes).is_ok() {
            let _ = std::process::Command::new("paplay").arg(&path).status();
        }
    });
}

fn ring_tone(hz: f32) -> Vec<u8> {
    let rate = 16000u32;
    let samples = rate / 2;
    let mut pcm = Vec::with_capacity(samples as usize * 2);
    for i in 0..samples {
        let t = i as f32 / rate as f32;
        let env = if i < 400 { i as f32 / 400.0 } else { 1.0 };
        let s = (t * hz * std::f32::consts::TAU).sin() * env * 0.25;
        pcm.extend_from_slice(&((s * i16::MAX as f32) as i16).to_le_bytes());
    }
    let data_len = pcm.len() as u32;
    let mut wav = Vec::with_capacity(44 + pcm.len());
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&rate.to_le_bytes());
    wav.extend_from_slice(&(rate * 2).to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    wav.extend_from_slice(&pcm);
    wav
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dbus_object_path_from_app_id() {
        assert_eq!(dbus_object_path("com.ghalbol.debug"), "/com/ghalbol/debug");
    }

    #[test]
    fn ring_wav_is_a_pcm_file() {
        let wav = ring_tone(880.0);
        assert!(wav.starts_with(b"RIFF"));
        assert!(wav.windows(4).any(|w| w == b"WAVE"));
        assert!(wav.len() > 44);
    }
}
