//! Audio codec abstraction for call media.
//!
//! The media engine is codec-agnostic. P0 ships [`NullCodec`] (lossless i16-LE
//! passthrough) so the pipeline + crypto + jitter logic can be unit-tested with
//! no native dependency. P1 adds a real Opus codec behind the same trait.

/// Call media is mono 48 kHz, 20 ms frames (matches Opus defaults).
pub const SAMPLE_RATE_HZ: u32 = 48_000;
pub const FRAME_MS: u32 = 20;
/// Samples per 20 ms mono frame (48000 / 1000 * 20 = 960).
pub const FRAME_SAMPLES: usize = (SAMPLE_RATE_HZ as usize / 1000) * FRAME_MS as usize;

/// Encode/decode one 20 ms mono frame. Encoder and decoder are separate trait
/// objects because stateful codecs (Opus) keep independent encoder/decoder state.
pub trait AudioCodec: Send {
    /// Encode exactly [`FRAME_SAMPLES`] i16 samples into a codec payload.
    fn encode(&mut self, pcm: &[i16]) -> Result<Vec<u8>, String>;

    /// Decode one frame into `out` (cleared first). `payload == None` means the
    /// packet was lost → conceal (PLC); the codec must still emit one frame.
    fn decode(&mut self, payload: Option<&[u8]>, out: &mut Vec<i16>) -> Result<(), String>;
}

/// Max Opus packet size we emit per 20 ms frame (generous upper bound).
const OPUS_MAX_PACKET: usize = 1500;

/// Opus encoder (Voip mode, in-band FEC) — production TX codec.
pub struct OpusEncoderCodec {
    enc: audiopus::coder::Encoder,
    buf: Vec<u8>,
}

impl OpusEncoderCodec {
    pub fn new() -> Result<Self, String> {
        use audiopus::{Application, Channels, SampleRate};
        let mut enc = audiopus::coder::Encoder::new(
            SampleRate::Hz48000,
            Channels::Mono,
            Application::Voip,
        )
        .map_err(|e| format!("opus encoder: {e}"))?;
        // Loss resilience: in-band FEC + an assumed packet-loss percentage.
        let _ = enc.set_inband_fec(true);
        let _ = enc.set_packet_loss_perc(10);
        Ok(Self { enc, buf: vec![0u8; OPUS_MAX_PACKET] })
    }
}

impl AudioCodec for OpusEncoderCodec {
    fn encode(&mut self, pcm: &[i16]) -> Result<Vec<u8>, String> {
        if pcm.len() != FRAME_SAMPLES {
            return Err(format!("frame must be {FRAME_SAMPLES} samples, got {}", pcm.len()));
        }
        let n = self
            .enc
            .encode(pcm, &mut self.buf)
            .map_err(|e| format!("opus encode: {e}"))?;
        Ok(self.buf[..n].to_vec())
    }

    fn decode(&mut self, _payload: Option<&[u8]>, _out: &mut Vec<i16>) -> Result<(), String> {
        Err("opus encoder cannot decode".to_string())
    }
}

/// Opus decoder (with PLC on packet loss) — production RX codec.
pub struct OpusDecoderCodec {
    dec: audiopus::coder::Decoder,
}

impl OpusDecoderCodec {
    pub fn new() -> Result<Self, String> {
        use audiopus::{Channels, SampleRate};
        let dec = audiopus::coder::Decoder::new(SampleRate::Hz48000, Channels::Mono)
            .map_err(|e| format!("opus decoder: {e}"))?;
        Ok(Self { dec })
    }
}

impl AudioCodec for OpusDecoderCodec {
    fn encode(&mut self, _pcm: &[i16]) -> Result<Vec<u8>, String> {
        Err("opus decoder cannot encode".to_string())
    }

    fn decode(&mut self, payload: Option<&[u8]>, out: &mut Vec<i16>) -> Result<(), String> {
        out.clear();
        out.resize(FRAME_SAMPLES, 0);
        let n = match payload {
            // PLC: None signals a lost packet to Opus's concealment.
            None => self
                .dec
                .decode(None::<&[u8]>, &mut out[..], false)
                .map_err(|e| format!("opus plc: {e}"))?,
            Some(p) => self
                .dec
                .decode(Some(p), &mut out[..], false)
                .map_err(|e| format!("opus decode: {e}"))?,
        };
        out.truncate(n);
        Ok(())
    }
}

/// Lossless i16-LE passthrough; concealment = silence. Test-only reference codec.
#[cfg(test)]
#[derive(Default)]
pub struct NullCodec;

#[cfg(test)]
impl AudioCodec for NullCodec {
    fn encode(&mut self, pcm: &[i16]) -> Result<Vec<u8>, String> {
        if pcm.len() != FRAME_SAMPLES {
            return Err(format!("frame must be {FRAME_SAMPLES} samples, got {}", pcm.len()));
        }
        let mut out = Vec::with_capacity(pcm.len() * 2);
        for s in pcm {
            out.extend_from_slice(&s.to_le_bytes());
        }
        Ok(out)
    }

    fn decode(&mut self, payload: Option<&[u8]>, out: &mut Vec<i16>) -> Result<(), String> {
        out.clear();
        match payload {
            None => out.resize(FRAME_SAMPLES, 0),
            Some(b) => {
                if b.len() != FRAME_SAMPLES * 2 {
                    return Err(format!("null payload {} bytes, want {}", b.len(), FRAME_SAMPLES * 2));
                }
                out.reserve(FRAME_SAMPLES);
                for ch in b.chunks_exact(2) {
                    out.push(i16::from_le_bytes([ch[0], ch[1]]));
                }
            }
        }
        Ok(())
    }
}
