//! Native call **video** media engine (codec-agnostic, transport-agnostic).
//!
//! Pipeline (see `docs/GHAL_BOL_VIDEO_NATIVE_V1.md`):
//! ```text
//! camera frame ─▶ encode(keyframe/delta) ─▶ fragment ─▶ seal (identity key)
//!              ─▶ wire chunks ─▶ [transport]
//! [transport]  ─▶ wire chunks ─▶ open ─▶ reassemble ─▶ jitter(keyframe-aware)
//!              ─▶ decode ─▶ render frame
//! ```
//! This module owns encode/decode, per-frame crypto, fragmentation/reassembly, and
//! the keyframe-aware jitter buffer. Camera capture, the render surface, and the
//! transport (libp2p substream `/ghal-bol/call-video/1.0.0` or QUIC datagrams) live
//! in separate layers and drive this engine — exactly mirroring the voice engine in
//! `call_media`.
//!
//! Crypto reuses `call_media::MediaCrypto` (AES-256-GCM, per-direction nonce) but is
//! keyed by a **distinct** video media key (different HKDF `info`), so audio and
//! video never share a `(key, nonce)` space.

#[cfg(target_os = "android")]
mod android_video;
mod capture;
mod codec;
#[cfg(not(target_arch = "wasm32"))]
mod codec_h264;
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
mod desktop_video;
mod jitter;
mod packet;
mod quality;
mod render;
mod session;

pub use capture::{desktop_capture_backend, spawn_camera_capture};
#[cfg(test)]
pub use codec::NullVideoCodec;
pub use codec::{RawVideoFrame, VideoDecoder, VideoEncoder};
#[cfg(not(target_arch = "wasm32"))]
pub use codec_h264::{H264Decoder, H264Encoder};
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
pub use desktop_video::push_camera_frame;
pub use session::{VideoControls, VideoStreams, run_video_session};

use crate::call_media::{MediaCrypto, MediaFrame};
use jitter::VideoJitter;
use packet::{Reassembler, VideoChunk, fragment_frame};

use std::collections::HashMap;
use std::sync::{Mutex as StdMutex, OnceLock};

/// Latest decoded **remote** frame per active call, for the FFI render pull.
struct FrameSlot {
    frame: RawVideoFrame,
    generation: u64,
}

fn remote_registry() -> &'static StdMutex<HashMap<String, FrameSlot>> {
    static REG: OnceLock<StdMutex<HashMap<String, FrameSlot>>> = OnceLock::new();
    REG.get_or_init(|| StdMutex::new(HashMap::new()))
}

fn local_registry() -> &'static StdMutex<HashMap<String, FrameSlot>> {
    static REG: OnceLock<StdMutex<HashMap<String, FrameSlot>>> = OnceLock::new();
    REG.get_or_init(|| StdMutex::new(HashMap::new()))
}

/// Publish a freshly decoded **remote** frame for `call_id` (session render loop).
pub fn publish_decoded_frame(call_id: &str, frame: RawVideoFrame) {
    let first = remote_registry()
        .lock()
        .ok()
        .map(|m| !m.contains_key(call_id))
        .unwrap_or(false);
    let next_gen = if let Ok(mut m) = remote_registry().lock() {
        let next_gen = m.get(call_id).map(|s| s.generation).unwrap_or(0) + 1;
        m.insert(
            call_id.to_string(),
            FrameSlot {
                frame: frame.clone(),
                generation: next_gen,
            },
        );
        next_gen
    } else {
        return;
    };
    render::publish_display_frame(call_id, "remote", &frame, next_gen);
    if first {
        crate::p2p::native_log::info(
            "call_video",
            format!(
                "first_remote_frame call_id={call_id} {}x{}",
                frame.width, frame.height
            ),
        );
    }
}

/// Publish the latest **local** camera preview frame (before encode).
pub fn publish_local_preview(call_id: &str, frame: RawVideoFrame) {
    let first = local_registry()
        .lock()
        .ok()
        .map(|m| !m.contains_key(call_id))
        .unwrap_or(false);
    let next_gen = if let Ok(mut m) = local_registry().lock() {
        let frame_gen = m.get(call_id).map(|s| s.generation).unwrap_or(0) + 1;
        m.insert(
            call_id.to_string(),
            FrameSlot {
                frame: frame.clone(),
                generation: frame_gen,
            },
        );
        frame_gen
    } else {
        return;
    };
    render::publish_display_frame(call_id, "local", &frame, next_gen);
    if first {
        crate::p2p::native_log::info(
            "call_video",
            format!(
                "first_local_preview call_id={call_id} {}x{}",
                frame.width, frame.height
            ),
        );
    }
}

/// Pull the latest decoded **remote** frame if newer than `since_generation`.
pub fn latest_decoded_frame(call_id: &str, since_generation: u64) -> Option<(RawVideoFrame, u64)> {
    pull_frame(remote_registry(), call_id, since_generation)
}

/// Pull the latest **local** preview frame if newer than `since_generation`.
pub fn latest_local_preview(call_id: &str, since_generation: u64) -> Option<(RawVideoFrame, u64)> {
    pull_frame(local_registry(), call_id, since_generation)
}

fn pull_frame(
    reg: &StdMutex<HashMap<String, FrameSlot>>,
    call_id: &str,
    since_generation: u64,
) -> Option<(RawVideoFrame, u64)> {
    let m = reg.lock().ok()?;
    let slot = m.get(call_id)?;
    if slot.generation <= since_generation {
        return None;
    }
    Some((slot.frame.clone(), slot.generation))
}

/// Convert a planar I420 frame to packed RGBA8888 natively (BT.601 full-range,
/// fixed-point). Moving this off the Flutter UI isolate is the main per-frame
/// latency win — Dart only feeds the result to `decodeImageFromPixels`.
pub fn i420_to_rgba(frame: &RawVideoFrame) -> Vec<u8> {
    let w = frame.width as usize;
    let h = frame.height as usize;
    let mut out = vec![0u8; w.saturating_mul(h).saturating_mul(4)];
    if w == 0 || h == 0 {
        return out;
    }
    let y_size = w * h;
    let uv_w = w / 2;
    let uv_h = h / 2;
    if frame.data.len() < y_size + 2 * uv_w * uv_h {
        return out;
    }
    let u_off = y_size;
    let v_off = y_size + uv_w * uv_h;
    let d = &frame.data;
    for row in 0..h {
        let uv_row = (row / 2).min(uv_h.saturating_sub(1));
        for col in 0..w {
            let uv_col = (col / 2).min(uv_w.saturating_sub(1));
            let y = d[row * w + col] as i32;
            let u = d[u_off + uv_row * uv_w + uv_col] as i32 - 128;
            let v = d[v_off + uv_row * uv_w + uv_col] as i32 - 128;
            // Fixed-point (<<16) BT.601 full-range — matches the old Dart float math.
            let r = y + ((91881 * v) >> 16);
            let g = y - ((22554 * u + 46802 * v) >> 16);
            let b = y + ((116130 * u) >> 16);
            let o = (row * w + col) * 4;
            out[o] = r.clamp(0, 255) as u8;
            out[o + 1] = g.clamp(0, 255) as u8;
            out[o + 2] = b.clamp(0, 255) as u8;
            out[o + 3] = 255;
        }
    }
    out
}

/// Downscale planar I420 so the longest edge is at most `max_edge` (nearest-neighbour).
pub fn i420_downscale_max_edge(frame: &RawVideoFrame, max_edge: u32) -> RawVideoFrame {
    let w = frame.width as usize;
    let h = frame.height as usize;
    if max_edge == 0 || frame.width.max(frame.height) <= max_edge {
        return frame.clone();
    }
    let step = ((frame.width.max(frame.height) + max_edge - 1) / max_edge).max(1) as usize;
    let dw = (w / step) & !1;
    let dh = (h / step) & !1;
    if dw == 0 || dh == 0 {
        return frame.clone();
    }
    let y_size = w * h;
    let uv_w = w / 2;
    let uv_h = h / 2;
    let u_off = y_size;
    let v_off = y_size + uv_w * uv_h;
    if frame.data.len() < y_size + 2 * uv_w * uv_h {
        return RawVideoFrame {
            width: dw as u32,
            height: dh as u32,
            data: vec![0u8; dw * dh + 2 * (dw / 2) * (dh / 2)],
        };
    }
    let d = &frame.data;
    let d_uv_w = dw / 2;
    let d_uv_h = dh / 2;
    let mut out = vec![0u8; dw * dh + 2 * d_uv_w * d_uv_h];
    let out_u = dw * dh;
    let out_v = out_u + d_uv_w * d_uv_h;
    for dy in 0..dh {
        let sy = (dy * step).min(h.saturating_sub(1));
        for dx in 0..dw {
            let sx = (dx * step).min(w.saturating_sub(1));
            out[dy * dw + dx] = d[sy * w + sx];
        }
    }
    for dy in 0..d_uv_h {
        let sy = (dy * step).min(uv_h.saturating_sub(1));
        for dx in 0..d_uv_w {
            let sx = (dx * step).min(uv_w.saturating_sub(1));
            out[out_u + dy * d_uv_w + dx] = d[u_off + sy * uv_w + sx];
            out[out_v + dy * d_uv_w + dx] = d[v_off + sy * uv_w + sx];
        }
    }
    RawVideoFrame {
        width: dw as u32,
        height: dh as u32,
        data: out,
    }
}

/// Like [`i420_to_rgba`] but downscales so the longest edge is at most `max_edge`
/// pixels (nearest-neighbour). Cuts socket/base64/decode cost ~4× at 360 vs 640.
pub fn i420_to_rgba_max_edge(frame: &RawVideoFrame, max_edge: u32) -> (Vec<u8>, u32, u32) {
    let w = frame.width as usize;
    let h = frame.height as usize;
    if max_edge == 0 || frame.width.max(frame.height) <= max_edge {
        return (i420_to_rgba(frame), frame.width, frame.height);
    }
    let step = ((frame.width.max(frame.height) + max_edge - 1) / max_edge).max(1) as usize;
    let dw = (w / step) & !1;
    let dh = (h / step) & !1;
    if dw == 0 || dh == 0 {
        return (i420_to_rgba(frame), frame.width, frame.height);
    }
    let y_size = w * h;
    let uv_w = w / 2;
    let uv_h = h / 2;
    let u_off = y_size;
    let v_off = y_size + uv_w * uv_h;
    if frame.data.len() < y_size + 2 * uv_w * uv_h {
        return (vec![0u8; dw * dh * 4], dw as u32, dh as u32);
    }
    let d = &frame.data;
    let mut out = vec![0u8; dw * dh * 4];
    for dy in 0..dh {
        let sy = (dy * step).min(h.saturating_sub(1));
        let uv_row = (sy / 2).min(uv_h.saturating_sub(1));
        for dx in 0..dw {
            let sx = (dx * step).min(w.saturating_sub(1));
            let uv_col = (sx / 2).min(uv_w.saturating_sub(1));
            let y = d[sy * w + sx] as i32;
            let u = d[u_off + uv_row * uv_w + uv_col] as i32 - 128;
            let v = d[v_off + uv_row * uv_w + uv_col] as i32 - 128;
            let r = y + ((91881 * v) >> 16);
            let g = y - ((22554 * u + 46802 * v) >> 16);
            let b = y + ((116130 * u) >> 16);
            let o = (dy * dw + dx) * 4;
            out[o] = r.clamp(0, 255) as u8;
            out[o + 1] = g.clamp(0, 255) as u8;
            out[o + 2] = b.clamp(0, 255) as u8;
            out[o + 3] = 255;
        }
    }
    (out, dw as u32, dh as u32)
}

/// Convert packed 8-bit RGBA/BGRA camera pixels (row stride `stride` bytes) to
/// planar I420 for the encoder. Done natively so the desktop capture path does no
/// per-pixel work on the Flutter UI isolate. `is_rgba` false means BGRA byte order.
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
pub fn packed_to_i420(
    src: &[u8],
    stride: usize,
    width: u32,
    height: u32,
    is_rgba: bool,
) -> Option<RawVideoFrame> {
    let w = (width as usize) & !1;
    let h = (height as usize) & !1;
    if w == 0 || h == 0 || stride < w * 4 {
        return None;
    }
    if src.len() < stride * h {
        return None;
    }
    let y_size = w * h;
    let uv_w = w / 2;
    let uv_h = h / 2;
    let mut out = vec![0u8; y_size + 2 * uv_w * uv_h];
    let u_off = y_size;
    let v_off = y_size + uv_w * uv_h;
    for y in 0..h {
        let row = y * stride;
        for x in 0..w {
            let p = row + x * 4;
            let (r, g, b) = if is_rgba {
                (src[p] as i32, src[p + 1] as i32, src[p + 2] as i32)
            } else {
                (src[p + 2] as i32, src[p + 1] as i32, src[p] as i32)
            };
            // Fixed-point (<<8) BT.601 full-range — matches the old Dart float math.
            out[y * w + x] = (((77 * r + 150 * g + 29 * b) >> 8).clamp(0, 255)) as u8;
            if y % 2 == 0 && x % 2 == 0 {
                let uv_idx = (y / 2) * uv_w + (x / 2);
                out[u_off + uv_idx] =
                    ((((-43 * r - 84 * g + 127 * b) >> 8) + 128).clamp(0, 255)) as u8;
                out[v_off + uv_idx] =
                    ((((127 * r - 106 * g - 21 * b) >> 8) + 128).clamp(0, 255)) as u8;
            }
        }
    }
    Some(RawVideoFrame {
        width: w as u32,
        height: h as u32,
        data: out,
    })
}


/// Shm path + dimensions for GPU texture registration (Flutter embedder).
pub fn texture_shm_info(call_id: &str, track: &str) -> Option<render::TextureShmInfo> {
    render::texture_shm_info(call_id, track)
}

pub(crate) fn track_call_shm(call_id: &str) {
    render::track_call(call_id);
}

/// Max in-flight partial frames during reassembly, and max buffered complete frames
/// in the jitter buffer. Video tolerates more buffering than audio but we keep this
/// bounded for latency + memory.
pub const DEFAULT_REASSEMBLY_PENDING: usize = 8;
/// Low depth for LAN P2P — deep buffers add latency without helping on a direct link.
pub const DEFAULT_VIDEO_JITTER_MAX: usize = 4;

/// The video engine for one active call. Not internally locked — the owner drives it
/// from the capture, network, and render paths (typically one task), like
/// `call_media::MediaEngine`.
pub struct VideoEngine {
    crypto: MediaCrypto,
    encoder: Box<dyn VideoEncoder>,
    decoder: Box<dyn VideoDecoder>,
    reasm: Reassembler,
    jitter: VideoJitter,
    chunk_data_bytes: usize,
    /// Monotonic per-direction chunk counter — the AES-GCM nonce counter.
    tx_chunk_seq: u64,
    /// Monotonic frame counter for ordering/reassembly.
    tx_frame_seq: u32,
    #[cfg(test)]
    rx_open_failures: u64,
}

impl VideoEngine {
    pub fn with_params(
        frame_key: &[u8; 32],
        local_is_a: bool,
        encoder: Box<dyn VideoEncoder>,
        decoder: Box<dyn VideoDecoder>,
        chunk_data_bytes: usize,
        reassembly_pending: usize,
        jitter_max: usize,
    ) -> Self {
        Self {
            crypto: MediaCrypto::new(frame_key, local_is_a),
            encoder,
            decoder,
            reasm: Reassembler::new(reassembly_pending),
            jitter: VideoJitter::new(jitter_max),
            chunk_data_bytes: chunk_data_bytes.max(1),
            tx_chunk_seq: 0,
            tx_frame_seq: 0,
            #[cfg(test)]
            rx_open_failures: 0,
        }
    }

    /// Capture path: encode one raw frame, fragment it, and seal each chunk into a
    /// wire packet to send. `force_keyframe` requests an intra frame (e.g. after a
    /// peer `key_request`).
    pub fn on_capture(
        &mut self,
        frame: &RawVideoFrame,
        force_keyframe: bool,
    ) -> Result<Vec<Vec<u8>>, String> {
        let encoded = self.encoder.encode(frame, force_keyframe)?;
        let frame_seq = self.tx_frame_seq;
        self.tx_frame_seq = self.tx_frame_seq.wrapping_add(1);
        let chunks = fragment_frame(frame_seq, &encoded, self.chunk_data_bytes);
        let mut wires = Vec::with_capacity(chunks.len());
        for chunk in chunks {
            let media = MediaFrame {
                seq: self.tx_chunk_seq,
                ts: frame_seq,
                flags: 0,
                payload: chunk.to_payload(),
            };
            self.tx_chunk_seq += 1;
            wires.push(self.crypto.seal(&media)?);
        }
        Ok(wires)
    }

    /// Network path: open one received wire chunk and feed it to reassembly. When a
    /// frame completes it is queued in the jitter buffer. Decrypt failures are
    /// counted and dropped (no plaintext leakage), matching the voice engine.
    pub fn on_wire(&mut self, wire: &[u8]) -> Result<(), String> {
        let media = match self.crypto.open(wire) {
            Ok(m) => m,
            Err(e) => {
                #[cfg(test)]
                {
                    self.rx_open_failures += 1;
                }
                return Err(e);
            }
        };
        let chunk = VideoChunk::from_payload(&media.payload)?;
        if let Some((frame_seq, encoded)) = self.reasm.push(chunk) {
            self.jitter.push(frame_seq, encoded);
        }
        Ok(())
    }

    /// Render tick: produce the next decoded frame to display, or `None` if nothing
    /// is renderable this tick (buffering / missing frame / waiting for keyframe).
    pub fn on_render(&mut self) -> Result<Option<RawVideoFrame>, String> {
        match self.jitter.pop() {
            Some(encoded) => self.decoder.decode(&encoded),
            None => Ok(None),
        }
    }

    pub fn set_bitrate_bps(&mut self, bps: u32) -> Result<(), String> {
        self.encoder.set_bitrate_bps(bps)
    }

    #[cfg(test)]
    pub fn open_failures(&self) -> u64 {
        self.rx_open_failures
    }
}

#[cfg(test)]
impl VideoEngine {
    pub fn take_keyframe_request(&mut self) -> bool {
        self.jitter.take_keyframe_request()
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn new_h264(frame_key: &[u8; 32], local_is_a: bool) -> Result<Self, String> {
        Ok(Self::with_params(
            frame_key,
            local_is_a,
            Box::new(codec_h264::H264Encoder::new()?),
            Box::new(codec_h264::H264Decoder::new()?),
            1024,
            DEFAULT_REASSEMBLY_PENDING,
            DEFAULT_VIDEO_JITTER_MAX,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::codec::EncodedVideoFrame;
    use super::*;

    fn key() -> [u8; 32] {
        let mut k = [0u8; 32];
        for (i, b) in k.iter_mut().enumerate() {
            *b = (i as u8).wrapping_mul(7).wrapping_add(1);
        }
        k
    }

    fn frame(w: u32, h: u32, fill: u8, len: usize) -> RawVideoFrame {
        RawVideoFrame {
            width: w,
            height: h,
            data: vec![fill; len],
        }
    }

    #[test]
    fn i420_to_rgba_grey_is_neutral_and_opaque() {
        // Y=128, U=V=128 (neutral chroma) → mid-grey, fully opaque.
        let (w, h) = (4usize, 4usize);
        let i420_len = w * h + 2 * (w / 2) * (h / 2);
        let mut data = vec![128u8; i420_len];
        // Make sure chroma planes are explicitly neutral.
        for b in data[w * h..].iter_mut() {
            *b = 128;
        }
        let rgba = i420_to_rgba(&RawVideoFrame {
            width: w as u32,
            height: h as u32,
            data,
        });
        assert_eq!(rgba.len(), w * h * 4);
        for px in rgba.chunks_exact(4) {
            assert!((px[0] as i32 - 128).abs() <= 1);
            assert!((px[1] as i32 - 128).abs() <= 1);
            assert!((px[2] as i32 - 128).abs() <= 1);
            assert_eq!(px[3], 255, "alpha must be opaque");
        }
    }

    #[test]
    fn i420_to_rgba_handles_undersized_input() {
        // Truncated data must not panic — returns a zeroed buffer of the right size.
        let rgba = i420_to_rgba(&RawVideoFrame {
            width: 8,
            height: 8,
            data: vec![0u8; 3],
        });
        assert_eq!(rgba.len(), 8 * 8 * 4);
    }

    #[test]
    fn i420_to_rgba_max_edge_downscales_640x480() {
        let (w, h) = (640usize, 480usize);
        let i420_len = w * h + 2 * (w / 2) * (h / 2);
        let frame = RawVideoFrame {
            width: w as u32,
            height: h as u32,
            data: vec![128u8; i420_len],
        };
        let (rgba, ow, oh) = i420_to_rgba_max_edge(&frame, 360);
        assert_eq!(ow, 320);
        assert_eq!(oh, 240);
        assert_eq!(rgba.len(), 320 * 240 * 4);
    }

    fn engine_a() -> VideoEngine {
        // Small chunk size so multi-chunk frames are exercised.
        VideoEngine::with_params(
            &key(),
            true,
            Box::new(NullVideoCodec),
            Box::new(NullVideoCodec),
            16,
            8,
            16,
        )
    }
    fn engine_b() -> VideoEngine {
        VideoEngine::with_params(
            &key(),
            false,
            Box::new(NullVideoCodec),
            Box::new(NullVideoCodec),
            16,
            8,
            16,
        )
    }

    #[test]
    fn round_trip_multichunk_frame() {
        let mut a = engine_a();
        let mut b = engine_b();
        // 100-byte payload at 16 bytes/chunk → 7 chunks (plus 8-byte null header).
        let f = frame(640, 480, 0xAB, 100);
        let wires = a.on_capture(&f, true).unwrap();
        assert!(wires.len() > 1, "frame must fragment into multiple chunks");
        for w in &wires {
            b.on_wire(w).unwrap();
        }
        let got = b.on_render().unwrap().expect("a frame should render");
        assert_eq!(got, f, "reassembled+decoded frame must match the original");
        assert_eq!(b.open_failures(), 0);
    }

    #[test]
    fn chunks_out_of_order_still_reassemble() {
        let mut a = engine_a();
        let mut b = engine_b();
        let f = frame(320, 240, 0x5C, 70);
        let mut wires = a.on_capture(&f, true).unwrap();
        wires.reverse(); // deliver chunks last-first
        for w in &wires {
            b.on_wire(w).unwrap();
        }
        assert_eq!(b.on_render().unwrap().unwrap(), f);
    }

    #[test]
    fn frames_render_in_order() {
        let mut a = engine_a();
        let mut b = engine_b();
        let f0 = frame(16, 16, 1, 20);
        let f1 = frame(16, 16, 2, 20);
        let f2 = frame(16, 16, 3, 20);
        for f in [&f0, &f1, &f2] {
            for w in &a.on_capture(f, true).unwrap() {
                b.on_wire(w).unwrap();
            }
        }
        assert_eq!(b.on_render().unwrap().unwrap(), f0);
        assert_eq!(b.on_render().unwrap().unwrap(), f1);
        assert_eq!(b.on_render().unwrap().unwrap(), f2);
    }

    #[test]
    fn dropped_chunk_drops_only_that_frame() {
        let mut a = engine_a();
        let mut b = engine_b();
        let f0 = frame(16, 16, 7, 50); // multi-chunk
        let f1 = frame(16, 16, 8, 50);
        let w0 = a.on_capture(&f0, true).unwrap();
        let w1 = a.on_capture(&f1, true).unwrap();
        // Deliver f0 missing its 2nd chunk; deliver all of f1.
        for (i, w) in w0.iter().enumerate() {
            if i == 1 {
                continue;
            }
            b.on_wire(w).unwrap();
        }
        for w in &w1 {
            b.on_wire(w).unwrap();
        }
        // f0 never completes; the buffer recovers to the next keyframe (f1).
        let rendered = b
            .on_render()
            .unwrap()
            .expect("f1 should render after f0 loss");
        assert_eq!(rendered, f1);
    }

    #[test]
    fn waits_for_keyframe_before_first_render() {
        // A real codec: only the first frame is a keyframe; the rest are deltas.
        struct DeltaCodec {
            sent_keyframe: bool,
        }
        impl VideoEncoder for DeltaCodec {
            fn encode(
                &mut self,
                frame: &RawVideoFrame,
                force: bool,
            ) -> Result<EncodedVideoFrame, String> {
                let keyframe = force || !self.sent_keyframe;
                self.sent_keyframe = true;
                let mut data = Vec::new();
                data.extend_from_slice(&frame.width.to_le_bytes());
                data.extend_from_slice(&frame.height.to_le_bytes());
                data.extend_from_slice(&frame.data);
                Ok(EncodedVideoFrame { keyframe, data })
            }
        }
        impl VideoDecoder for DeltaCodec {
            fn decode(&mut self, f: &EncodedVideoFrame) -> Result<Option<RawVideoFrame>, String> {
                let w = u32::from_le_bytes(f.data[0..4].try_into().unwrap());
                let h = u32::from_le_bytes(f.data[4..8].try_into().unwrap());
                Ok(Some(RawVideoFrame {
                    width: w,
                    height: h,
                    data: f.data[8..].to_vec(),
                }))
            }
        }
        let mut a = VideoEngine::with_params(
            &key(),
            true,
            Box::new(DeltaCodec {
                sent_keyframe: false,
            }),
            Box::new(DeltaCodec {
                sent_keyframe: false,
            }),
            64,
            8,
            16,
        );
        let mut b = VideoEngine::with_params(
            &key(),
            false,
            Box::new(DeltaCodec {
                sent_keyframe: false,
            }),
            Box::new(DeltaCodec {
                sent_keyframe: false,
            }),
            64,
            8,
            16,
        );
        let kf = frame(16, 16, 1, 10); // keyframe
        let delta = frame(16, 16, 2, 10); // delta
        let wkf = a.on_capture(&kf, false).unwrap();
        let wdelta = a.on_capture(&delta, false).unwrap();
        // Deliver only the delta first → nothing renders (no keyframe yet) + request raised.
        for w in &wdelta {
            b.on_wire(w).unwrap();
        }
        assert!(
            b.on_render().unwrap().is_none(),
            "must not render a delta before a keyframe"
        );
        assert!(
            b.take_keyframe_request(),
            "should request a keyframe while stalled"
        );
        // Now deliver the keyframe → it renders.
        for w in &wkf {
            b.on_wire(w).unwrap();
        }
        assert_eq!(b.on_render().unwrap().unwrap(), kf);
    }

    #[test]
    fn tampered_wire_is_rejected() {
        let mut a = engine_a();
        let mut b = engine_b();
        let mut wires = a.on_capture(&frame(8, 8, 4, 10), true).unwrap();
        let last = wires[0].len() - 1;
        wires[0][last] ^= 0xFF; // corrupt GCM tag of the first chunk
        assert!(b.on_wire(&wires[0]).is_err());
        assert_eq!(b.open_failures(), 1);
    }

    /// Full pipeline with the real H.264 codec across two engines: encode → fragment
    /// → seal → wire → open → reassemble → keyframe-aware jitter → decode → render.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn h264_full_pipeline_two_engines() {
        let (w, h) = (96u32, 64u32);
        fn i420(w: u32, h: u32, t: u8) -> RawVideoFrame {
            let (w, h) = (w as usize, h as usize);
            let mut data = vec![128u8; w * h + 2 * ((w / 2) * (h / 2))];
            for (i, b) in data[..w * h].iter_mut().enumerate() {
                *b = (i as u8).wrapping_add(t);
            }
            RawVideoFrame {
                width: w as u32,
                height: h as u32,
                data,
            }
        }
        let mut a = VideoEngine::new_h264(&key(), true).expect("h264 a");
        let mut b = VideoEngine::new_h264(&key(), false).expect("h264 b");

        let mut rendered = None;
        for t in 0..3u8 {
            // First frame forces a keyframe so the receiver can start.
            let wires = a.on_capture(&i420(w, h, t * 20), t == 0).unwrap();
            for wire in &wires {
                b.on_wire(wire).unwrap();
            }
            if let Some(frame) = b.on_render().unwrap() {
                rendered = Some(frame);
            }
        }
        let frame = rendered.expect("a frame should decode through the full pipeline");
        assert_eq!((frame.width, frame.height), (w, h));
        assert_eq!(b.open_failures(), 0);
    }

    #[test]
    fn wrong_key_cannot_open() {
        let mut a = engine_a();
        let mut other = key();
        other[0] ^= 0xFF;
        let mut b = VideoEngine::with_params(
            &other,
            false,
            Box::new(NullVideoCodec),
            Box::new(NullVideoCodec),
            16,
            8,
            16,
        );
        let wires = a.on_capture(&frame(8, 8, 9, 10), true).unwrap();
        for w in &wires {
            assert!(b.on_wire(w).is_err());
        }
    }
}
