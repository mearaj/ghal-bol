//! Desktop (Linux / macOS / Windows): Flutter captures the camera in the UI process
//! (PipeWire / AVFoundation / MediaFoundation) and pushes I420 frames via RPC.
//!
//! Mirrors [`android_video`]: the daemon video engine receives frames on the same
//! `mpsc` channel that nokhwa used to fill.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use tokio::sync::mpsc;

use super::session::VideoControls;
use super::RawVideoFrame;

static FRAME_TX: OnceLock<Mutex<Option<mpsc::Sender<RawVideoFrame>>>> = OnceLock::new();
static CAPTURE_ACTIVE: AtomicBool = AtomicBool::new(false);
static INJECT_FRAMES_RX: AtomicU64 = AtomicU64::new(0);

fn frame_tx() -> &'static Mutex<Option<mpsc::Sender<RawVideoFrame>>> {
    FRAME_TX.get_or_init(|| Mutex::new(None))
}

/// Inject one I420 frame from the Flutter UI (daemon JSON-RPC / in-process FFI).
pub fn push_camera_frame(frame: RawVideoFrame) {
    if !CAPTURE_ACTIVE.load(Ordering::Relaxed) {
        return;
    }
    let w = frame.width;
    let h = frame.height;
    if w == 0 || h == 0 || w % 2 != 0 || h % 2 != 0 {
        return;
    }
    let expected = (w as usize) * (h as usize) + 2 * ((w as usize / 2) * (h as usize / 2));
    if frame.data.len() < expected {
        return;
    }
    if let Ok(guard) = frame_tx().lock() {
        if let Some(tx) = guard.as_ref() {
            let _ = tx.try_send(frame);
            let n = INJECT_FRAMES_RX.fetch_add(1, Ordering::Relaxed) + 1;
            if n == 1 {
                crate::p2p::native_log::info(
                    "call_video",
                    format!("flutter first camera frame {w}x{h}"),
                );
            }
        }
    }
}

/// Start receiving UI-pushed frames into an async channel for the video engine.
pub fn spawn(controls: VideoControls) -> Result<mpsc::Receiver<RawVideoFrame>, String> {
    let (tx, rx) = mpsc::channel::<RawVideoFrame>(4);
    if let Ok(mut g) = frame_tx().lock() {
        *g = Some(tx);
    }
    CAPTURE_ACTIVE.store(true, Ordering::Relaxed);
    INJECT_FRAMES_RX.store(0, Ordering::Relaxed);
    tokio::spawn(async move {
        while !controls.is_stopped() {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
        stop_capture();
    });
    Ok(rx)
}

/// Tear down injected capture (session stop).
pub fn stop_capture() {
    CAPTURE_ACTIVE.store(false, Ordering::Relaxed);
    INJECT_FRAMES_RX.store(0, Ordering::Relaxed);
    if let Ok(mut g) = frame_tx().lock() {
        g.take();
    }
    super::capture::reset_desktop_capture_backend();
}
