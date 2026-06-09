//! Video codec abstraction for the native call video engine.
//!
//! Mirrors `call_media::codec::AudioCodec`: the engine is codec-agnostic and drives
//! a boxed encoder/decoder. Concrete impls wrap proven crates (e.g. `cros-codecs`,
//! `yscv-video`, `rav1e`+`rav1d`, `openh264`, or platform HW `MediaCodec`/VideoToolbox).
//! `NullVideoCodec` is a passthrough used by unit tests to exercise the
//! packetizer/jitter/crypto without a real codec — exactly like `NullCodec` for voice.

/// One raw (decoded / to-be-encoded) video frame. The engine never interprets
/// pixels; `data` is whatever the codec layer agreed on (e.g. I420/NV12).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawVideoFrame {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

/// One encoded video frame (a single coded picture). `keyframe` marks an
/// intra-coded frame that can be decoded without any prior frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncodedVideoFrame {
    pub keyframe: bool,
    pub data: Vec<u8>,
}

pub trait VideoEncoder: Send {
    /// Encode one raw frame. `force_keyframe` requests an intra frame (after loss
    /// or on a peer `key_request`).
    fn encode(
        &mut self,
        frame: &RawVideoFrame,
        force_keyframe: bool,
    ) -> Result<EncodedVideoFrame, String>;

    /// Adjust target bitrate (adaptive quality). Default: no-op.
    fn set_bitrate_bps(&mut self, _bps: u32) -> Result<(), String> {
        Ok(())
    }
}

pub trait VideoDecoder: Send {
    /// Decode one encoded frame. `Ok(None)` means the frame could not produce a
    /// picture yet (e.g. a delta frame decoded before any keyframe) — never an error.
    fn decode(&mut self, frame: &EncodedVideoFrame) -> Result<Option<RawVideoFrame>, String>;
}

/// Passthrough codec for tests: each "encoded" frame is self-contained (carries
/// its own dimensions), so every frame is effectively a keyframe and round-trips
/// byte-for-byte. Lets the engine's transport/jitter/crypto be tested in isolation.
#[cfg(test)]
pub struct NullVideoCodec;

#[cfg(test)]
impl VideoEncoder for NullVideoCodec {
    fn encode(
        &mut self,
        frame: &RawVideoFrame,
        _force_keyframe: bool,
    ) -> Result<EncodedVideoFrame, String> {
        let mut data = Vec::with_capacity(8 + frame.data.len());
        data.extend_from_slice(&frame.width.to_le_bytes());
        data.extend_from_slice(&frame.height.to_le_bytes());
        data.extend_from_slice(&frame.data);
        Ok(EncodedVideoFrame { keyframe: true, data })
    }
}

#[cfg(test)]
impl VideoDecoder for NullVideoCodec {
    fn decode(&mut self, frame: &EncodedVideoFrame) -> Result<Option<RawVideoFrame>, String> {
        if frame.data.len() < 8 {
            return Err("null video frame too short".to_string());
        }
        let width = u32::from_le_bytes(frame.data[0..4].try_into().unwrap());
        let height = u32::from_le_bytes(frame.data[4..8].try_into().unwrap());
        Ok(Some(RawVideoFrame { width, height, data: frame.data[8..].to_vec() }))
    }
}
