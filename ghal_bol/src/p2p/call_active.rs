//! In-process snapshot of the active voice/video call for UI re-sync after process restart.
//!
//! `:p2p` / the daemon may outlive the Flutter UI on Android; this registry lets the
//! shell restore the call screen instead of leaving the user unaware of an ongoing call.

use std::sync::{Mutex, OnceLock};

#[derive(Debug, Clone)]
pub struct ActiveCallSnapshot {
    pub call_id: String,
    pub peer_public_key_hex: String,
    pub voice_active: bool,
    pub video_active: bool,
    pub camera_on: bool,
    pub remote_video_on: bool,
}

fn slot() -> &'static Mutex<Option<ActiveCallSnapshot>> {
    static REG: OnceLock<Mutex<Option<ActiveCallSnapshot>>> = OnceLock::new();
    REG.get_or_init(|| Mutex::new(None))
}

pub fn snapshot() -> Option<ActiveCallSnapshot> {
    slot().lock().ok().and_then(|g| g.clone())
}

pub fn clear() {
    if let Ok(mut g) = slot().lock() {
        *g = None;
    }
}

pub fn on_voice_start(call_id: &str, peer_public_key_hex: &str) {
    let Ok(mut g) = slot().lock() else {
        return;
    };
    let mut snap = g.clone().unwrap_or(ActiveCallSnapshot {
        call_id: call_id.to_string(),
        peer_public_key_hex: peer_public_key_hex.to_string(),
        voice_active: false,
        video_active: false,
        camera_on: false,
        remote_video_on: false,
    });
    snap.call_id = call_id.to_string();
    snap.peer_public_key_hex = peer_public_key_hex.to_string();
    snap.voice_active = true;
    *g = Some(snap);
}

pub fn on_voice_stop(call_id: &str) {
    let Ok(mut g) = slot().lock() else {
        return;
    };
    let Some(snap) = g.as_mut() else {
        return;
    };
    if snap.call_id != call_id {
        return;
    }
    snap.voice_active = false;
    if !snap.video_active {
        *g = None;
    }
}

pub fn on_video_start(call_id: &str, peer_public_key_hex: &str, camera_enabled: bool) {
    let Ok(mut g) = slot().lock() else {
        return;
    };
    let mut snap = g.clone().unwrap_or(ActiveCallSnapshot {
        call_id: call_id.to_string(),
        peer_public_key_hex: peer_public_key_hex.to_string(),
        voice_active: false,
        video_active: false,
        camera_on: false,
        remote_video_on: false,
    });
    snap.call_id = call_id.to_string();
    snap.peer_public_key_hex = peer_public_key_hex.to_string();
    snap.video_active = true;
    snap.camera_on = camera_enabled;
    *g = Some(snap);
}

pub fn on_video_stop(call_id: &str) {
    let Ok(mut g) = slot().lock() else {
        return;
    };
    let Some(snap) = g.as_mut() else {
        return;
    };
    if snap.call_id != call_id {
        return;
    }
    snap.video_active = false;
    snap.camera_on = false;
    snap.remote_video_on = false;
    if !snap.voice_active {
        *g = None;
    }
}

pub fn set_camera_on(call_id: &str, on: bool) {
    let Ok(mut g) = slot().lock() else {
        return;
    };
    let Some(snap) = g.as_mut() else {
        return;
    };
    if snap.call_id == call_id {
        snap.camera_on = on;
    }
}

pub fn set_remote_video_on(call_id: &str, on: bool) {
    let Ok(mut g) = slot().lock() else {
        return;
    };
    let Some(snap) = g.as_mut() else {
        return;
    };
    if snap.call_id == call_id {
        snap.remote_video_on = on;
    }
}
