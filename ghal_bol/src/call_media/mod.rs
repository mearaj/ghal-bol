//! Native call **voice** media engine (codec-agnostic, transport-agnostic).
//!
//! Pipeline (see `docs/GHAL_BOL_CALL_NATIVE_V2.md`):
//! ```text
//! capture PCM ─▶ encode ─▶ seal (identity key) ─▶ wire bytes ─▶ [transport]
//! [transport] ─▶ wire bytes ─▶ open ─▶ jitter buffer ─▶ decode(PLC) ─▶ playout PCM
//! ```
//! This module owns encode/decode, per-frame crypto, and the jitter buffer.
//! Audio device I/O (capture/playback, AEC) and the transport (libp2p substream
//! or QUIC datagrams) live in separate layers and drive this engine.

mod android_audio;
mod audio_device;
mod codec;
mod crypto;
mod jitter;
mod session;

pub use audio_device::default_audio_backend;
#[cfg(target_os = "android")]
pub use audio_device::set_android_audio_ready;

/// True after `:p2p` JNI init (`initAndroidAudio`) — video capture reuses the same Context.
#[cfg(target_os = "android")]
pub fn android_p2p_context_ready() -> bool {
    audio_device::is_android_audio_ready()
}
#[cfg(target_os = "android")]
pub use android_audio::{
    ensure_voice_audio_mode, reset_voice_audio_mode_flag, set_speakerphone,
};
#[cfg(not(target_os = "android"))]
pub use android_audio::set_speakerphone;
pub use codec::{
    AudioCodec, OpusDecoderCodec, OpusEncoderCodec, FRAME_MS, FRAME_SAMPLES, SAMPLE_RATE_HZ,
};
#[cfg(test)]
pub use codec::NullCodec;
pub use jitter::Playout;
pub use session::{run_media_session, MediaControls};

pub(crate) use crypto::MediaCrypto;
use jitter::JitterBuffer;

/// One decoded/encoded media frame moving through the engine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaFrame {
    /// Monotonic per-sender counter; also the AES-GCM nonce counter and the
    /// jitter-buffer ordering key.
    pub seq: u64,
    /// Sample-clock timestamp (`seq * FRAME_SAMPLES`) for A/V sync + diagnostics.
    pub ts: u32,
    /// Reserved bit flags (DTX/comfort-noise markers, etc.).
    pub flags: u8,
    /// Codec payload.
    pub payload: Vec<u8>,
}

/// Default prebuffer / cap (in 20 ms frames): 4 → 80 ms prebuffer, 16 → 320 ms cap.
pub const DEFAULT_JITTER_TARGET: usize = 4;
pub const DEFAULT_JITTER_MAX: usize = 16;

/// True when the local identity sorts lexicographically lower than the peer —
/// used to pick opposite crypto directions on the two sides (see [`crypto`]).
pub fn local_is_a(local_public_key_hex: &str, peer_public_key_hex: &str) -> bool {
    local_public_key_hex.trim().to_ascii_lowercase()
        < peer_public_key_hex.trim().to_ascii_lowercase()
}

/// The voice engine for one active call. Not internally locked — the owner drives
/// it from the capture, network, and playout paths (typically one task).
pub struct MediaEngine {
    crypto: MediaCrypto,
    encoder: Box<dyn AudioCodec>,
    decoder: Box<dyn AudioCodec>,
    jitter: JitterBuffer,
    tx_seq: u64,
    #[cfg(test)]
    rx_count: u64,
    #[cfg(test)]
    rx_open_failures: u64,
}

impl MediaEngine {
    pub fn new(
        frame_key: &[u8; 32],
        local_is_a: bool,
        encoder: Box<dyn AudioCodec>,
        decoder: Box<dyn AudioCodec>,
    ) -> Self {
        Self::with_jitter(
            frame_key,
            local_is_a,
            encoder,
            decoder,
            DEFAULT_JITTER_TARGET,
            DEFAULT_JITTER_MAX,
        )
    }

    /// Production engine using Opus (Voip + FEC) encoder/decoder.
    pub fn new_opus(frame_key: &[u8; 32], local_is_a: bool) -> Result<Self, String> {
        let encoder = Box::new(OpusEncoderCodec::new()?);
        let decoder = Box::new(OpusDecoderCodec::new()?);
        Ok(Self::new(frame_key, local_is_a, encoder, decoder))
    }

    pub fn with_jitter(
        frame_key: &[u8; 32],
        local_is_a: bool,
        encoder: Box<dyn AudioCodec>,
        decoder: Box<dyn AudioCodec>,
        jitter_target: usize,
        jitter_max: usize,
    ) -> Self {
        Self {
            crypto: MediaCrypto::new(frame_key, local_is_a),
            encoder,
            decoder,
            jitter: JitterBuffer::new(jitter_target, jitter_max),
            tx_seq: 0,
            #[cfg(test)]
            rx_count: 0,
            #[cfg(test)]
            rx_open_failures: 0,
        }
    }

    /// Capture path: encode + seal one 20 ms PCM frame into wire bytes to send.
    pub fn on_capture(&mut self, pcm: &[i16]) -> Result<Vec<u8>, String> {
        let payload = self.encoder.encode(pcm)?;
        let seq = self.tx_seq;
        self.tx_seq += 1;
        let frame = MediaFrame {
            seq,
            ts: (seq as u32).wrapping_mul(FRAME_SAMPLES as u32),
            flags: 0,
            payload,
        };
        self.crypto.seal(&frame)
    }

    /// Network path: open a received wire packet and enqueue it for playout.
    /// Decryption failures are counted and dropped (no plaintext leakage).
    pub fn on_wire(&mut self, wire: &[u8]) -> Result<(), String> {
        match self.crypto.open(wire) {
            Ok(frame) => {
                #[cfg(test)]
                {
                    self.rx_count += 1;
                }
                self.jitter.push(frame);
                Ok(())
            }
            Err(e) => {
                #[cfg(test)]
                {
                    self.rx_open_failures += 1;
                }
                Err(e)
            }
        }
    }

    /// Playout tick (every 20 ms): produce the next PCM frame for the speaker.
    pub fn on_playout(&mut self, out: &mut Vec<i16>) -> Result<(), String> {
        match self.jitter.pop() {
            Playout::Frame(f) => self.decoder.decode(Some(&f.payload), out),
            Playout::Conceal => self.decoder.decode(None, out),
            Playout::Silence => {
                out.clear();
                out.resize(FRAME_SAMPLES, 0);
                Ok(())
            }
        }
    }

    #[cfg(test)]
    pub fn frames_received(&self) -> u64 {
        self.rx_count
    }

    #[cfg(test)]
    pub fn open_failures(&self) -> u64 {
        self.rx_open_failures
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> [u8; 32] {
        let mut k = [0u8; 32];
        for (i, b) in k.iter_mut().enumerate() {
            *b = i as u8;
        }
        k
    }

    fn ramp(base: i16) -> Vec<i16> {
        (0..FRAME_SAMPLES).map(|i| base.wrapping_add(i as i16)).collect()
    }

    fn engine_a() -> MediaEngine {
        MediaEngine::new(&key(), true, Box::new(NullCodec), Box::new(NullCodec))
    }
    fn engine_b() -> MediaEngine {
        MediaEngine::new(&key(), false, Box::new(NullCodec), Box::new(NullCodec))
    }

    #[test]
    fn round_trip_in_order() {
        let mut a = engine_a();
        let mut b = engine_b();
        // A sends enough frames to fill B's prebuffer.
        let mut wires = Vec::new();
        for i in 0..(DEFAULT_JITTER_TARGET + 2) {
            wires.push(a.on_capture(&ramp(i as i16)).unwrap());
        }
        for w in &wires {
            b.on_wire(w).unwrap();
        }
        // First DEFAULT_JITTER_TARGET-ish ticks: once started, frames come out in order.
        let mut got = Vec::new();
        let mut out = Vec::new();
        for _ in 0..(DEFAULT_JITTER_TARGET + 2) {
            b.on_playout(&mut out).unwrap();
            got.push(out.clone());
        }
        // At least the first sent frame must reappear intact among playout.
        assert!(got.iter().any(|f| *f == ramp(0)));
        assert!(got.iter().any(|f| *f == ramp(1)));
        assert_eq!(b.open_failures(), 0);
    }

    #[test]
    fn reorder_is_sorted_on_playout() {
        let mut a = engine_a();
        let mut b = engine_b();
        let w: Vec<Vec<u8>> = (0..6).map(|i| a.on_capture(&ramp(i as i16 * 100)).unwrap()).collect();
        // Deliver out of order.
        for idx in [2usize, 0, 1, 4, 3, 5] {
            b.on_wire(&w[idx]).unwrap();
        }
        let mut out = Vec::new();
        let mut seq_pcm = Vec::new();
        for _ in 0..6 {
            b.on_playout(&mut out).unwrap();
            if out != vec![0i16; FRAME_SAMPLES] {
                seq_pcm.push(out.clone());
            }
        }
        // Frames 0..n appear in ascending order (no shuffling survived).
        let first = seq_pcm.iter().position(|f| *f == ramp(0)).unwrap();
        let second = seq_pcm.iter().position(|f| *f == ramp(100)).unwrap();
        assert!(first < second);
    }

    #[test]
    fn missing_frame_triggers_conceal_then_continues() {
        let mut a = engine_a();
        let mut b = engine_b();
        let w: Vec<Vec<u8>> = (0..(DEFAULT_JITTER_TARGET + 3))
            .map(|i| a.on_capture(&ramp(i as i16 + 1)).unwrap())
            .collect();
        // Drop seq 1 (index 1); deliver the rest.
        for (i, wire) in w.iter().enumerate() {
            if i == 1 {
                continue;
            }
            b.on_wire(wire).unwrap();
        }
        let mut out = Vec::new();
        let mut produced = Vec::new();
        for _ in 0..w.len() {
            b.on_playout(&mut out).unwrap();
            produced.push(out.clone());
        }
        // Concealment for the dropped frame = silence (NullCodec), but later
        // frames still play, proving the buffer advanced past the gap.
        assert!(produced.iter().any(|f| *f == ramp(1))); // seq 0 (base 1)
        assert!(produced.iter().any(|f| *f == ramp(3))); // seq 2 (base 3) survived the gap
    }

    #[test]
    fn tampered_wire_is_rejected() {
        let mut a = engine_a();
        let mut b = engine_b();
        let mut wire = a.on_capture(&ramp(7)).unwrap();
        let last = wire.len() - 1;
        wire[last] ^= 0xFF; // corrupt the GCM tag
        assert!(b.on_wire(&wire).is_err());
        assert_eq!(b.frames_received(), 0);
        assert_eq!(b.open_failures(), 1);
    }

    #[test]
    fn wrong_key_cannot_open() {
        let mut a = engine_a();
        let mut other = [0u8; 32];
        other[0] = 0xAA;
        let mut b = MediaEngine::new(&other, false, Box::new(NullCodec), Box::new(NullCodec));
        let wire = a.on_capture(&ramp(9)).unwrap();
        assert!(b.on_wire(&wire).is_err());
    }

    #[test]
    fn opposite_directions_use_distinct_nonces() {
        // Same key, both sending seq 0 — different dir byte must yield different wire.
        let mut a = engine_a();
        let mut b = engine_b();
        let wa = a.on_capture(&ramp(0)).unwrap();
        let wb = b.on_capture(&ramp(0)).unwrap();
        assert_ne!(wa[0], wb[0], "direction byte must differ");
        assert_ne!(wa, wb, "ciphertext must differ across directions");
    }

    #[test]
    fn local_is_a_orders_by_pubkey() {
        assert!(local_is_a("02aa", "03bb"));
        assert!(!local_is_a("03bb", "02aa"));
    }

    #[test]
    fn opus_engine_round_trip_runs() {
        let mut a = MediaEngine::new_opus(&key(), true).expect("opus a");
        let mut b = MediaEngine::new_opus(&key(), false).expect("opus b");
        let tone: Vec<i16> = (0..FRAME_SAMPLES)
            .map(|i| ((i as f32 * 0.05).sin() * 8000.0) as i16)
            .collect();
        let wires: Vec<Vec<u8>> = (0..(DEFAULT_JITTER_TARGET + 2))
            .map(|_| a.on_capture(&tone).unwrap())
            .collect();
        for w in &wires {
            b.on_wire(w).unwrap();
        }
        let mut out = Vec::new();
        let mut decoded_any = false;
        for _ in 0..(DEFAULT_JITTER_TARGET + 2) {
            b.on_playout(&mut out).unwrap();
            assert_eq!(out.len(), FRAME_SAMPLES, "opus frame must be one 20 ms frame");
            if out.iter().any(|&s| s != 0) {
                decoded_any = true;
            }
        }
        assert!(decoded_any, "opus should decode non-silent audio");
        assert_eq!(b.open_failures(), 0);
        // Opus compresses: a 20 ms speech frame is far smaller than raw PCM.
        assert!(wires[0].len() < FRAME_SAMPLES * 2);
    }
}
