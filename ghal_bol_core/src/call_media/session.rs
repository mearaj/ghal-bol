//! Real-time media session: glues an [`MediaEngine`] to an audio backend and a
//! transport, both behind traits so the same loop runs on desktop (cpal),
//! Android (AAudio), and in headless tests (mock backend + loopback channels).
//!
//! Threading model (matches cpal/AAudio callbacks):
//! * the audio backend **pushes** captured 20 ms PCM frames onto `capture_rx`
//!   and **pulls** playout frames from `playout_tx`;
//! * the transport bridges wire bytes via `wire_out` (engine → peer) and
//!   `wire_in` (peer → engine).
//!
//! The session task owns the [`MediaEngine`] and runs the 20 ms playout clock.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use tokio::sync::mpsc;

use super::{FRAME_MS, FRAME_SAMPLES, MediaEngine};

/// Channels handed back by an [`AudioBackend`] when capture/playback start.
pub struct AudioStreams {
    /// Captured 20 ms mono frames (`FRAME_SAMPLES` i16) from the microphone.
    pub capture_rx: mpsc::Receiver<Vec<i16>>,
    /// Frames to render to the speaker (20 ms mono).
    pub playout_tx: mpsc::Sender<Vec<i16>>,
}

/// Platform audio capture + playback. Real impls: cpal (desktop), AAudio (Android).
pub trait AudioBackend: Send {
    fn start(&mut self) -> Result<AudioStreams, String>;
    fn stop(&mut self);
}

/// Shared mute flag — when set, captured frames are replaced with silence before
/// encoding (keeps the stream/clock alive so the peer's jitter buffer is steady).
#[derive(Clone, Default)]
pub struct MediaControls {
    pub mic_muted: Arc<AtomicBool>,
    pub stop: Arc<AtomicBool>,
    /// Encoded+sealed frames handed to the transport (observable by FFI/logs).
    pub frames_sent: Arc<AtomicU64>,
    /// Sealed frames opened from the transport (observable by FFI/logs).
    pub frames_received: Arc<AtomicU64>,
}

impl MediaControls {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn set_mic_muted(&self, muted: bool) {
        self.mic_muted.store(muted, Ordering::Relaxed);
    }
    pub fn request_stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
    pub fn is_stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }
    pub fn sent(&self) -> u64 {
        self.frames_sent.load(Ordering::Relaxed)
    }
    pub fn received(&self) -> u64 {
        self.frames_received.load(Ordering::Relaxed)
    }
}

/// Run one call's media until `controls.stop` is set or a channel closes.
///
/// `wire_out` carries sealed packets to the transport writer; `wire_in` delivers
/// received sealed packets from the transport reader.
pub async fn run_media_session(
    mut engine: MediaEngine,
    mut audio: AudioStreams,
    wire_out: mpsc::Sender<Vec<u8>>,
    mut wire_in: mpsc::Receiver<Vec<u8>>,
    controls: MediaControls,
) {
    let mut playout = tokio::time::interval(Duration::from_millis(FRAME_MS as u64));
    playout.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut stop_poll = tokio::time::interval(Duration::from_millis(100));
    let mut out = Vec::with_capacity(FRAME_SAMPLES);
    let silence = vec![0i16; FRAME_SAMPLES];

    loop {
        if controls.stop.load(Ordering::Relaxed) {
            break;
        }
        tokio::select! {
            // Mic → encode/seal → transport.
            cap = audio.capture_rx.recv() => {
                match cap {
                    Some(frame) => {
                        let src = if controls.mic_muted.load(Ordering::Relaxed) {
                            &silence
                        } else {
                            &frame
                        };
                        if src.len() == FRAME_SAMPLES {
                            if let Ok(wire) = engine.on_capture(src) {
                                // Drop-oldest: if the transport is backed up, skip
                                // rather than block the audio path (latency control).
                                if wire_out.try_send(wire).is_ok() {
                                    controls.frames_sent.fetch_add(1, Ordering::Relaxed);
                                }
                            }
                        }
                    }
                    None => break, // capture ended
                }
            }
            // Transport → open/unseal → jitter buffer.
            w = wire_in.recv() => {
                match w {
                    Some(bytes) => {
                        if engine.on_wire(&bytes).is_ok() {
                            controls.frames_received.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    None => break, // transport closed
                }
            }
            // 20 ms playout clock: jitter → decode/PLC → speaker.
            _ = playout.tick() => {
                if engine.on_playout(&mut out).is_ok() {
                    let _ = audio.playout_tx.try_send(out.clone());
                }
            }
            _ = stop_poll.tick() => {}
        }
    }
    audio.playout_tx.closed().await;
}

/// Test/headless audio backend: feeds preset capture frames, collects playout.
#[cfg(test)]
pub struct MockAudioBackend {
    capture_frames: Vec<Vec<i16>>,
    played: Arc<std::sync::Mutex<Vec<Vec<i16>>>>,
}

#[cfg(test)]
impl MockAudioBackend {
    pub fn new(capture_frames: Vec<Vec<i16>>) -> (Self, Arc<std::sync::Mutex<Vec<Vec<i16>>>>) {
        let played = Arc::new(std::sync::Mutex::new(Vec::new()));
        (
            Self {
                capture_frames,
                played: Arc::clone(&played),
            },
            played,
        )
    }
}

#[cfg(test)]
impl AudioBackend for MockAudioBackend {
    fn start(&mut self) -> Result<AudioStreams, String> {
        let (cap_tx, capture_rx) = mpsc::channel::<Vec<i16>>(64);
        let (playout_tx, mut playout_rx) = mpsc::channel::<Vec<i16>>(256);
        let frames = std::mem::take(&mut self.capture_frames);
        tokio::spawn(async move {
            for f in frames {
                if cap_tx.send(f).await.is_err() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(FRAME_MS as u64)).await;
            }
            // keep sender alive a bit so the session doesn't see capture-close early
            tokio::time::sleep(Duration::from_millis(500)).await;
            drop(cap_tx);
        });
        let played = Arc::clone(&self.played);
        tokio::spawn(async move {
            while let Some(f) = playout_rx.recv().await {
                if let Ok(mut g) = played.lock() {
                    g.push(f);
                }
            }
        });
        Ok(AudioStreams {
            capture_rx,
            playout_tx,
        })
    }
    fn stop(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::call_media::{MediaEngine, NullCodec};

    fn key() -> [u8; 32] {
        let mut k = [0u8; 32];
        for (i, b) in k.iter_mut().enumerate() {
            *b = (i as u8).wrapping_mul(7).wrapping_add(1);
        }
        k
    }

    fn tone(base: i16) -> Vec<i16> {
        (0..FRAME_SAMPLES)
            .map(|i| base.wrapping_add((i % 64) as i16 * 100))
            .collect()
    }

    #[tokio::test]
    async fn two_sessions_exchange_audio_over_loopback() {
        // Loopback wire channels between A and B.
        let (a2b_tx, a2b_rx) = mpsc::channel::<Vec<u8>>(256);
        let (b2a_tx, b2a_rx) = mpsc::channel::<Vec<u8>>(256);

        // A captures 12 tone frames; B captures nothing.
        let a_frames: Vec<Vec<i16>> = (0..12).map(|i| tone(i as i16 * 10)).collect();
        let (mut a_backend, _a_played) = MockAudioBackend::new(a_frames);
        let (mut b_backend, b_played) = MockAudioBackend::new(Vec::new());
        let a_audio = a_backend.start().unwrap();
        let b_audio = b_backend.start().unwrap();

        let a_eng = MediaEngine::new(&key(), true, Box::new(NullCodec), Box::new(NullCodec));
        let b_eng = MediaEngine::new(&key(), false, Box::new(NullCodec), Box::new(NullCodec));
        let a_ctl = MediaControls::new();
        let b_ctl = MediaControls::new();

        let a = tokio::spawn(run_media_session(
            a_eng,
            a_audio,
            a2b_tx,
            b2a_rx,
            a_ctl.clone(),
        ));
        let b = tokio::spawn(run_media_session(
            b_eng,
            b_audio,
            b2a_tx,
            a2b_rx,
            b_ctl.clone(),
        ));

        tokio::time::sleep(Duration::from_millis(600)).await;
        a_ctl.request_stop();
        b_ctl.request_stop();
        let _ = tokio::time::timeout(Duration::from_secs(2), a).await;
        let _ = tokio::time::timeout(Duration::from_secs(2), b).await;

        let played = b_played.lock().unwrap();
        // B must have produced playout frames, and at least one must be the
        // audio A sent (non-silent) — proving capture→wire→jitter→playout works.
        assert!(!played.is_empty(), "B produced no playout frames");
        assert!(
            played.iter().any(|f| f.iter().any(|&s| s != 0)),
            "B never played non-silent audio from A",
        );
    }
}
