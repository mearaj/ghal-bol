//! H.264 video codec (OpenH264) implementing [`VideoEncoder`]/[`VideoDecoder`].
//!
//! H.264 is the proven realtime video-call codec (baseline profile, low latency)
//! with near-universal hardware decode. This is the first concrete codec behind the
//! engine's swappable trait; AV1 (`rav1e`+`rav1d`) and platform HW (`MediaCodec` /
//! VAAPI / VideoToolbox) slot in later without touching the engine or transport.
//!
//! Frame format: `RawVideoFrame.data` is planar **I420** (YUV 4:2:0):
//! `Y (w*h) || U (w/2 * h/2) || V (w/2 * h/2)`. Width/height must be even.

use openh264::decoder::Decoder;
use openh264::encoder::{BitRate, Encoder, EncoderConfig, FrameRate, IntraFramePeriod};
use openh264::formats::YUVSource;
use openh264::OpenH264API;

/// Default target bitrate / frame rate for a realtime call. Congestion control will
/// drive these adaptively later (see docs); fixed for the first working version.
pub const DEFAULT_BITRATE_BPS: u32 = 2_000_000;
const DEFAULT_FPS: f32 = 30.0;
/// Periodic intra (key) frame interval. The receiver's keyframe-aware jitter buffer
/// recovers from packet loss by jumping to the next keyframe, so a steady keyframe
/// cadence bounds recovery time without needing an explicit peer key-request path.
const INTRA_PERIOD_FRAMES: u32 = 60; // ~2 s at 30 fps

use super::codec::{EncodedVideoFrame, RawVideoFrame, VideoDecoder, VideoEncoder};

/// Borrowed I420 view passed to the OpenH264 encoder (no copy).
struct I420View<'a> {
    width: usize,
    height: usize,
    data: &'a [u8],
}

impl I420View<'_> {
    fn chroma(&self) -> (usize, usize) {
        (self.width / 2, self.height / 2)
    }
}

impl YUVSource for I420View<'_> {
    fn dimensions(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    fn strides(&self) -> (usize, usize, usize) {
        (self.width, self.width / 2, self.width / 2)
    }

    fn y(&self) -> &[u8] {
        &self.data[..self.width * self.height]
    }

    fn u(&self) -> &[u8] {
        let (cw, ch) = self.chroma();
        let off = self.width * self.height;
        &self.data[off..off + cw * ch]
    }

    fn v(&self) -> &[u8] {
        let (cw, ch) = self.chroma();
        let off = self.width * self.height + cw * ch;
        &self.data[off..off + cw * ch]
    }
}

fn i420_len(width: usize, height: usize) -> usize {
    width * height + 2 * ((width / 2) * (height / 2))
}

/// Scan an Annex-B byte stream for an IDR slice (NAL type 5) or SPS (type 7),
/// which mark a decodable-from-scratch keyframe. Independent of the codec crate's
/// frame-type API, so it stays correct across versions.
fn annexb_has_keyframe(buf: &[u8]) -> bool {
    let mut i = 0;
    while i + 3 < buf.len() {
        let start = if buf[i] == 0 && buf[i + 1] == 0 && buf[i + 2] == 1 {
            Some(3)
        } else if i + 4 <= buf.len()
            && buf[i] == 0
            && buf[i + 1] == 0
            && buf[i + 2] == 0
            && buf[i + 3] == 1
        {
            Some(4)
        } else {
            None
        };
        match start {
            Some(len) => {
                let nal_idx = i + len;
                if nal_idx < buf.len() {
                    let nal_unit_type = buf[nal_idx] & 0x1F;
                    if nal_unit_type == 5 || nal_unit_type == 7 {
                        return true;
                    }
                }
                i = nal_idx + 1;
            }
            None => i += 1,
        }
    }
    false
}

pub struct H264Encoder {
    enc: Encoder,
}

impl H264Encoder {
    pub fn new() -> Result<Self, String> {
        Self::with_bitrate_bps(DEFAULT_BITRATE_BPS)
    }

    pub fn with_bitrate_bps(bps: u32) -> Result<Self, String> {
        let config = EncoderConfig::new()
            .bitrate(BitRate::from_bps(bps.max(300_000)))
            .max_frame_rate(FrameRate::from_hz(DEFAULT_FPS))
            .intra_frame_period(IntraFramePeriod::from_num_frames(INTRA_PERIOD_FRAMES));
        let enc = Encoder::with_api_config(OpenH264API::from_source(), config)
            .map_err(|e| format!("openh264 encoder init: {e}"))?;
        Ok(Self { enc })
    }

    pub fn set_bitrate_bps(&mut self, bps: u32) -> Result<(), String> {
        *self = Self::with_bitrate_bps(bps)?;
        Ok(())
    }
}

impl VideoEncoder for H264Encoder {
    fn encode(
        &mut self,
        frame: &RawVideoFrame,
        force_keyframe: bool,
    ) -> Result<EncodedVideoFrame, String> {
        let w = frame.width as usize;
        let h = frame.height as usize;
        if w == 0 || h == 0 || w % 2 != 0 || h % 2 != 0 {
            return Err("h264: width/height must be non-zero and even".to_string());
        }
        let expected = i420_len(w, h);
        if frame.data.len() != expected {
            return Err(format!(
                "h264: expected {expected} I420 bytes for {w}x{h}, got {}",
                frame.data.len()
            ));
        }
        if force_keyframe {
            self.enc.force_intra_frame();
        }
        let src = I420View { width: w, height: h, data: &frame.data };
        let bitstream = self.enc.encode(&src).map_err(|e| format!("h264 encode: {e}"))?;
        let data = bitstream.to_vec();
        let keyframe = force_keyframe || annexb_has_keyframe(&data);
        Ok(EncodedVideoFrame { keyframe, data })
    }

    fn set_bitrate_bps(&mut self, bps: u32) -> Result<(), String> {
        H264Encoder::set_bitrate_bps(self, bps)
    }
}

pub struct H264Decoder {
    dec: Decoder,
}

impl H264Decoder {
    pub fn new() -> Result<Self, String> {
        let dec = Decoder::with_api_config(OpenH264API::from_source(), Default::default())
            .map_err(|e| format!("openh264 decoder init: {e}"))?;
        Ok(Self { dec })
    }
}

impl VideoDecoder for H264Decoder {
    fn decode(&mut self, frame: &EncodedVideoFrame) -> Result<Option<RawVideoFrame>, String> {
        let decoded = self
            .dec
            .decode(&frame.data)
            .map_err(|e| format!("h264 decode: {e}"))?;
        let Some(yuv) = decoded else {
            return Ok(None);
        };
        let (w, h) = yuv.dimensions();
        let (sy, su, sv) = yuv.strides();
        let (cw, ch) = (w / 2, h / 2);
        let mut data = Vec::with_capacity(i420_len(w, h));
        let yb = yuv.y();
        for row in 0..h {
            let off = row * sy;
            data.extend_from_slice(&yb[off..off + w]);
        }
        let ub = yuv.u();
        for row in 0..ch {
            let off = row * su;
            data.extend_from_slice(&ub[off..off + cw]);
        }
        let vb = yuv.v();
        for row in 0..ch {
            let off = row * sv;
            data.extend_from_slice(&vb[off..off + cw]);
        }
        Ok(Some(RawVideoFrame { width: w as u32, height: h as u32, data }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient_i420(w: usize, h: usize, t: u8) -> RawVideoFrame {
        let mut data = vec![0u8; i420_len(w, h)];
        for y in 0..h {
            for x in 0..w {
                data[y * w + x] = ((x + y) as u8).wrapping_add(t);
            }
        }
        let off = w * h;
        let (cw, ch) = (w / 2, h / 2);
        for c in 0..cw * ch {
            data[off + c] = 128;
            data[off + cw * ch + c] = 128;
        }
        RawVideoFrame { width: w as u32, height: h as u32, data }
    }

    #[test]
    fn h264_encode_marks_keyframe_and_decodes() {
        let mut enc = H264Encoder::new().expect("encoder");
        let mut dec = H264Decoder::new().expect("decoder");
        let (w, h) = (64, 48);

        let first = enc.encode(&gradient_i420(w, h, 0), false).expect("encode 1");
        assert!(first.keyframe, "first H.264 frame must be a keyframe (SPS/PPS/IDR)");
        assert!(!first.data.is_empty());

        // Decode the keyframe (carries SPS/PPS) — must yield a picture of the right size.
        let mut got = dec.decode(&first).expect("decode 1");
        if got.is_none() {
            // Feed one more frame; some decoders need a follow-up NAL to emit.
            let second = enc.encode(&gradient_i420(w, h, 10), false).expect("encode 2");
            got = dec.decode(&second).expect("decode 2");
        }
        let frame = got.expect("a decoded frame");
        assert_eq!((frame.width, frame.height), (w as u32, h as u32));
        assert_eq!(frame.data.len(), i420_len(w, h));
    }

    #[test]
    fn h264_compresses_below_raw() {
        let mut enc = H264Encoder::new().expect("encoder");
        let (w, h) = (128, 96);
        let raw = i420_len(w, h);
        let f = enc.encode(&gradient_i420(w, h, 0), true).expect("encode");
        assert!(f.data.len() < raw, "encoded frame should be smaller than raw I420");
    }

    #[test]
    fn h264_rejects_bad_dimensions() {
        let mut enc = H264Encoder::new().expect("encoder");
        let bad = RawVideoFrame { width: 65, height: 48, data: vec![0u8; 10] };
        assert!(enc.encode(&bad, false).is_err());
    }
}
