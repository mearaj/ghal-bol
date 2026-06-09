//! Real-time **video** session: glues a [`VideoEngine`] to a capture/render backend
//! and a transport, both behind traits — mirroring `call_media::session`.
//!
//! Threading model:
//! * the capture backend **pushes** raw camera frames (I420) onto `capture_rx`;
//! * decoded frames are **pushed** to `render_tx` for the display backend;
//! * the transport bridges sealed wire chunks via `wire_out` (engine → peer) and
//!   `wire_in` (peer → engine).
//!
//! The session task owns the [`VideoEngine`] and runs the render clock. Like the
//! voice session, all device I/O (camera, display) lives in the backend, so the
//! same loop runs on desktop (nokhwa), Android (Camera2), and headless tests.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;

use super::quality::AdaptiveQuality;
use super::{RawVideoFrame, VideoEngine};

/// Channels between capture, engine, and render for one video session.
pub struct VideoStreams {
    /// Raw camera frames (I420) from the capture device.
    pub capture_rx: mpsc::Receiver<RawVideoFrame>,
    /// Decoded frames to hand to the display/render surface.
    pub render_tx: mpsc::Sender<RawVideoFrame>,
}


/// Shared control flags for one video session (observable by FFI/logs).
#[derive(Clone, Default)]
pub struct VideoControls {
    /// When set, captured frames are dropped (camera off) — the peer simply
    /// receives no new frames and holds the last one.
    pub camera_off: Arc<AtomicBool>,
    pub stop: Arc<AtomicBool>,
    /// Forces the next captured frame to be encoded as a keyframe.
    pub force_keyframe: Arc<AtomicBool>,
    /// Source frames encoded+sent.
    pub frames_sent: Arc<AtomicU64>,
    /// Frames decoded+rendered.
    pub frames_received: Arc<AtomicU64>,
}

impl VideoControls {
    pub fn new() -> Self {
        let c = Self::default();
        // Camera off until the user toggles video on (receive-only decode still runs).
        c.camera_off.store(true, Ordering::Relaxed);
        c.force_keyframe.store(true, Ordering::Relaxed);
        c
    }
    pub fn set_camera_off(&self, off: bool) {
        self.camera_off.store(off, Ordering::Relaxed);
    }
    pub fn request_stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
    pub fn is_stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }
    #[cfg(test)]
    pub fn sent(&self) -> u64 {
        self.frames_sent.load(Ordering::Relaxed)
    }
}

/// Poll interval for the render/decode clock (~60 Hz). Video is event-driven by
/// arriving frames; this just drains the jitter buffer steadily.
const RENDER_TICK_MS: u64 = 16;

/// Run one call's video until `controls.stop` is set or a channel closes.
pub async fn run_video_session(
    mut engine: VideoEngine,
    mut video: VideoStreams,
    call_id: String,
    wire_out: mpsc::Sender<Vec<u8>>,
    mut wire_in: mpsc::Receiver<Vec<u8>>,
    controls: VideoControls,
) {
    let mut render = tokio::time::interval(Duration::from_millis(RENDER_TICK_MS));
    render.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut stop_poll = tokio::time::interval(Duration::from_millis(100));
    let mut adaptive = AdaptiveQuality::new();

    loop {
        if controls.stop.load(Ordering::Relaxed) {
            break;
        }
        tokio::select! {
            // Camera → encode/seal → transport (each frame may be several chunks).
            cap = video.capture_rx.recv() => {
                match cap {
                    Some(frame) => {
                        if controls.camera_off.load(Ordering::Relaxed) {
                            continue;
                        }
                        crate::call_video::publish_local_preview(&call_id, frame.clone());
                        let encode_frame = adaptive.frame_for_encode(&frame);
                        if controls.frames_sent.load(Ordering::Relaxed) == 0 {
                            crate::p2p::native_log::info(
                                "call_video",
                                format!(
                                    "first_local_frame call_id={call_id} cap={}x{} encode={}x{}",
                                    frame.width,
                                    frame.height,
                                    encode_frame.width,
                                    encode_frame.height
                                ),
                            );
                        }
                        let force = controls.force_keyframe.swap(false, Ordering::Relaxed);
                        match engine.on_capture(&encode_frame, force) {
                            Ok(wires) => {
                                let mut drops = 0u32;
                                let total = wires.len() as u32;
                                for w in wires {
                                    // Drop-oldest if the transport is backed up — never
                                    // block the capture path (latency control).
                                    if wire_out.try_send(w).is_err() {
                                        drops += 1;
                                    }
                                }
                                adaptive.note_wire_send(drops, total);
                                controls.frames_sent.fetch_add(1, Ordering::Relaxed);
                            }
                            Err(_) => {
                                // Encoder hiccup — force a keyframe on the next frame.
                                controls.force_keyframe.store(true, Ordering::Relaxed);
                            }
                        }
                    }
                    None => break, // capture ended
                }
            }
            // Transport → open/unseal → reassemble → jitter.
            w = wire_in.recv() => {
                match w {
                    Some(bytes) => { let _ = engine.on_wire(&bytes); }
                    None => break, // transport closed
                }
            }
            // Render clock: jitter → decode → display.
            _ = render.tick() => {
                if let Ok(Some(frame)) = engine.on_render() {
                    if video.render_tx.try_send(frame).is_ok() {
                        controls.frames_received.fetch_add(1, Ordering::Relaxed);
                    }
                }
                // Note: receiver-side keyframe recovery currently relies on the
                // encoder's periodic intra frames (see codec config). The engine's
                // `take_keyframe_request()` flag is reserved for an explicit
                // peer key-request signal (future optimization).
            }
            _ = stop_poll.tick() => {
                if let Some(bps) = adaptive.tick() {
                    let _ = engine.set_bitrate_bps(bps);
                    controls.force_keyframe.store(true, Ordering::Relaxed);
                }
            }
        }
    }
    video.render_tx.closed().await;
    #[cfg(target_os = "android")]
    crate::call_video::android_video::stop_capture();
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    crate::call_video::desktop_video::stop_capture();
}

/// Platform video capture + render surface. Headless/tests use [`MockVideoBackend`].
#[cfg(test)]
trait VideoCaptureBackend: Send {
    fn start(&mut self) -> Result<VideoStreams, String>;
}

/// Test/headless backend: feeds preset capture frames, collects rendered frames.
#[cfg(test)]
pub struct MockVideoBackend {
    capture_frames: Vec<RawVideoFrame>,
    rendered: Arc<std::sync::Mutex<Vec<RawVideoFrame>>>,
}

#[cfg(test)]
impl MockVideoBackend {
    pub fn new(
        capture_frames: Vec<RawVideoFrame>,
    ) -> (Self, Arc<std::sync::Mutex<Vec<RawVideoFrame>>>) {
        let rendered = Arc::new(std::sync::Mutex::new(Vec::new()));
        (Self { capture_frames, rendered: Arc::clone(&rendered) }, rendered)
    }
}

#[cfg(test)]
impl VideoCaptureBackend for MockVideoBackend {
    fn start(&mut self) -> Result<VideoStreams, String> {
        let (cap_tx, capture_rx) = mpsc::channel::<RawVideoFrame>(8);
        let (render_tx, mut render_rx) = mpsc::channel::<RawVideoFrame>(16);
        let frames = std::mem::take(&mut self.capture_frames);
        tokio::spawn(async move {
            for f in frames {
                if cap_tx.send(f).await.is_err() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(33)).await;
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
            drop(cap_tx);
        });
        let rendered = Arc::clone(&self.rendered);
        tokio::spawn(async move {
            while let Some(f) = render_rx.recv().await {
                if let Ok(mut g) = rendered.lock() {
                    g.push(f);
                }
            }
        });
        Ok(VideoStreams { capture_rx, render_tx })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::call_video::{NullVideoCodec, VideoEngine};

    fn key() -> [u8; 32] {
        let mut k = [0u8; 32];
        for (i, b) in k.iter_mut().enumerate() {
            *b = (i as u8).wrapping_mul(7).wrapping_add(1);
        }
        k
    }

    fn frame(tag: u8) -> RawVideoFrame {
        RawVideoFrame { width: 32, height: 24, data: vec![tag; 32 * 24] }
    }

    fn engine(local_is_a: bool) -> VideoEngine {
        VideoEngine::with_params(
            &key(),
            local_is_a,
            Box::new(NullVideoCodec),
            Box::new(NullVideoCodec),
            64,
            8,
            16,
        )
    }

    #[tokio::test]
    async fn two_sessions_exchange_video_over_loopback() {
        let (a2b_tx, a2b_rx) = mpsc::channel::<Vec<u8>>(256);
        let (b2a_tx, b2a_rx) = mpsc::channel::<Vec<u8>>(256);

        let a_frames: Vec<RawVideoFrame> = (1..=8).map(frame).collect();
        let (mut a_backend, _a_rendered) = MockVideoBackend::new(a_frames);
        let (mut b_backend, b_rendered) = MockVideoBackend::new(Vec::new());
        let a_video = a_backend.start().unwrap();
        let b_video = b_backend.start().unwrap();

        let a_ctl = VideoControls::new();
        a_ctl.set_camera_off(false);
        let b_ctl = VideoControls::new();
        let a = tokio::spawn(run_video_session(
            engine(true),
            a_video,
            "test-a".to_string(),
            a2b_tx,
            b2a_rx,
            a_ctl.clone(),
        ));
        let b = tokio::spawn(run_video_session(
            engine(false),
            b_video,
            "test-b".to_string(),
            b2a_tx,
            a2b_rx,
            b_ctl.clone(),
        ));

        tokio::time::sleep(Duration::from_millis(700)).await;
        a_ctl.request_stop();
        b_ctl.request_stop();
        let _ = tokio::time::timeout(Duration::from_secs(2), a).await;
        let _ = tokio::time::timeout(Duration::from_secs(2), b).await;

        let rendered = b_rendered.lock().unwrap();
        assert!(!rendered.is_empty(), "B rendered no frames from A");
        // The exact frames A captured must reappear (NullVideoCodec is lossless).
        assert!(rendered.iter().any(|f| *f == frame(1)), "first frame missing");
        assert!(rendered.iter().any(|f| *f == frame(8)), "later frame missing");
        assert!(a_ctl.sent() >= 8, "A should have sent its captured frames");
    }
}
