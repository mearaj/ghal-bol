//! Voice-note inner schema + Opus pack/unpack for DM `MsgKind::Voice`.
//!
//! Same rail as text (transport KEM seal in `msg_v1`). **Not** call media keys.
//! Limits: ≤120 s, sealed envelope budget ≤3 MB (see [VOICE_MESSAGES_PLAN.md]).

use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
use serde::{Deserialize, Serialize};

#[cfg(not(target_arch = "wasm32"))]
use crate::call_media::{
    AudioCodec, FRAME_SAMPLES, OpusDecoderCodec, OpusEncoderCodec, SAMPLE_RATE_HZ,
};

/// Product max duration for a voice note (2 minutes).
pub const VOICE_MAX_DURATION_MS: u32 = 120_000;

/// Hard stop before seal — leave headroom under DM 4 MB frame limit.
pub const VOICE_MAX_SEALED_INNER_BYTES: usize = 3 * 1024 * 1024;

pub const VOICE_MSG_VERSION: u32 = 1;
pub const VOICE_CODEC: &str = "opus";
pub const VOICE_CHANNELS: u8 = 1;

#[cfg(target_arch = "wasm32")]
const SAMPLE_RATE_HZ: u32 = 48_000;

/// Inner plaintext JSON before transport KEM seal.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VoiceInner {
    pub voice_msg_version: u32,
    pub codec: String,
    pub duration_ms: u32,
    pub sample_rate_hz: u32,
    pub channels: u8,
    pub audio_b64: String,
}

impl VoiceInner {
    pub fn validate(&self) -> Result<(), String> {
        if self.voice_msg_version != VOICE_MSG_VERSION {
            return Err(format!(
                "unsupported voice_msg_version={}",
                self.voice_msg_version
            ));
        }
        if self.codec != VOICE_CODEC {
            return Err(format!("unsupported voice codec={}", self.codec));
        }
        if self.channels != VOICE_CHANNELS {
            return Err("voice channels must be 1".to_string());
        }
        if self.sample_rate_hz != SAMPLE_RATE_HZ {
            return Err(format!(
                "voice sample_rate_hz must be {SAMPLE_RATE_HZ}, got {}",
                self.sample_rate_hz
            ));
        }
        if self.duration_ms == 0 {
            return Err("voice duration_ms must be > 0".to_string());
        }
        if self.duration_ms > VOICE_MAX_DURATION_MS {
            return Err(format!("voice duration exceeds {VOICE_MAX_DURATION_MS} ms"));
        }
        if self.audio_b64.trim().is_empty() {
            return Err("voice audio_b64 empty".to_string());
        }
        Ok(())
    }

    pub fn opus_bytes(&self) -> Result<Vec<u8>, String> {
        B64.decode(self.audio_b64.trim())
            .map_err(|e| format!("audio_b64: {e}"))
    }

    pub fn to_json_bytes(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        let bytes = serde_json::to_vec(self).map_err(|e| format!("voice inner json: {e}"))?;
        if bytes.len() > VOICE_MAX_SEALED_INNER_BYTES {
            return Err(format!(
                "voice inner exceeds {VOICE_MAX_SEALED_INNER_BYTES} bytes"
            ));
        }
        Ok(bytes)
    }

    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > VOICE_MAX_SEALED_INNER_BYTES {
            return Err("voice inner too large".to_string());
        }
        let inner: Self =
            serde_json::from_slice(bytes).map_err(|e| format!("voice inner json: {e}"))?;
        inner.validate()?;
        Ok(inner)
    }
}

/// Pack PCM i16 mono 48 kHz into length-prefixed Opus packets: `[u16 LE len][packet]…`
#[cfg(not(target_arch = "wasm32"))]
pub fn encode_pcm_to_opus_blob(pcm: &[i16]) -> Result<Vec<u8>, String> {
    if pcm.is_empty() {
        return Err("empty pcm".to_string());
    }
    let max_samples =
        (VOICE_MAX_DURATION_MS as usize / 1000) * SAMPLE_RATE_HZ as usize + FRAME_SAMPLES;
    if pcm.len() > max_samples {
        return Err(format!("pcm exceeds max duration ({} samples)", pcm.len()));
    }
    let mut enc = OpusEncoderCodec::new()?;
    let mut out = Vec::new();
    let mut offset = 0usize;
    let mut frame = vec![0i16; FRAME_SAMPLES];
    while offset < pcm.len() {
        let end = (offset + FRAME_SAMPLES).min(pcm.len());
        frame.fill(0);
        frame[..end - offset].copy_from_slice(&pcm[offset..end]);
        let pkt = enc.encode(&frame)?;
        if pkt.len() > u16::MAX as usize {
            return Err("opus packet too large".to_string());
        }
        out.extend_from_slice(&(pkt.len() as u16).to_le_bytes());
        out.extend_from_slice(&pkt);
        offset += FRAME_SAMPLES;
    }
    Ok(out)
}

/// Decode length-prefixed Opus blob back to PCM i16 mono 48 kHz.
#[cfg(not(target_arch = "wasm32"))]
pub fn decode_opus_blob_to_pcm(blob: &[u8]) -> Result<Vec<i16>, String> {
    let mut dec = OpusDecoderCodec::new()?;
    let mut out = Vec::new();
    let mut frame = Vec::new();
    let mut i = 0usize;
    while i + 2 <= blob.len() {
        let len = u16::from_le_bytes([blob[i], blob[i + 1]]) as usize;
        i += 2;
        if i + len > blob.len() {
            return Err("truncated opus blob".to_string());
        }
        let pkt = &blob[i..i + len];
        i += len;
        dec.decode(Some(pkt), &mut frame)?;
        out.extend_from_slice(&frame);
    }
    if out.is_empty() {
        return Err("empty opus decode".to_string());
    }
    Ok(out)
}

pub fn duration_ms_from_pcm_len(pcm_samples: usize) -> u32 {
    let ms = (pcm_samples as u64 * 1000) / SAMPLE_RATE_HZ as u64;
    ms.min(u64::from(VOICE_MAX_DURATION_MS)) as u32
}

pub fn build_voice_inner(duration_ms: u32, opus_blob: &[u8]) -> Result<VoiceInner, String> {
    if duration_ms == 0 || duration_ms > VOICE_MAX_DURATION_MS {
        return Err(format!("duration_ms must be 1..={VOICE_MAX_DURATION_MS}"));
    }
    if opus_blob.is_empty() {
        return Err("opus blob empty".to_string());
    }
    let inner = VoiceInner {
        voice_msg_version: VOICE_MSG_VERSION,
        codec: VOICE_CODEC.to_string(),
        duration_ms,
        sample_rate_hz: SAMPLE_RATE_HZ,
        channels: VOICE_CHANNELS,
        audio_b64: B64.encode(opus_blob),
    };
    let _ = inner.to_json_bytes()?;
    Ok(inner)
}

pub fn voice_preview(duration_ms: u32) -> String {
    let total_secs = (duration_ms.saturating_add(999) / 1000).max(1);
    format!(
        "Voice message {mins}:{secs:02}",
        mins = total_secs / 60,
        secs = total_secs % 60
    )
}

/// Read little-endian i16 PCM from a mono WAV (48 kHz preferred) or raw PCM file.
pub fn read_pcm_i16_le_file(path: &std::path::Path) -> Result<Vec<i16>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read pcm: {e}"))?;
    parse_wav_or_raw_pcm(&bytes)
}

fn parse_wav_or_raw_pcm(bytes: &[u8]) -> Result<Vec<i16>, String> {
    if bytes.len() >= 44 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WAVE" {
        let mut i = 12usize;
        while i + 8 <= bytes.len() {
            let id = &bytes[i..i + 4];
            let sz = u32::from_le_bytes([bytes[i + 4], bytes[i + 5], bytes[i + 6], bytes[i + 7]])
                as usize;
            i += 8;
            if id == b"data" {
                let end = (i + sz).min(bytes.len());
                return pcm_bytes_to_i16(&bytes[i..end]);
            }
            i = (i + sz).min(bytes.len());
            if !sz.is_multiple_of(2) {
                i = (i + 1).min(bytes.len());
            }
        }
        return Err("wav missing data chunk".to_string());
    }
    pcm_bytes_to_i16(bytes)
}

fn pcm_bytes_to_i16(bytes: &[u8]) -> Result<Vec<i16>, String> {
    if bytes.len() < 2 || !bytes.len().is_multiple_of(2) {
        return Err("pcm byte length must be even and non-empty".to_string());
    }
    let mut out = Vec::with_capacity(bytes.len() / 2);
    for chunk in bytes.chunks_exact(2) {
        out.push(i16::from_le_bytes([chunk[0], chunk[1]]));
    }
    Ok(out)
}

/// Persist Opus blob under `{ui_data_dir}/voice/{message_id}.opus`.
pub fn voice_audio_path(
    app_namespace: &str,
    message_id: &str,
) -> Result<std::path::PathBuf, String> {
    let cfg = crate::app_paths::storage_config_for_namespace(app_namespace);
    let mut dir = crate::app_paths::ui_data_dir(&cfg).map_err(|e| format!("{e}"))?;
    dir.push("voice");
    std::fs::create_dir_all(&dir).map_err(|e| format!("voice dir: {e}"))?;
    let mid = message_id.trim();
    if mid.is_empty() || mid.contains('/') || mid.contains('\\') || mid.contains("..") {
        return Err("invalid message_id for voice path".to_string());
    }
    dir.push(format!("{mid}.opus"));
    Ok(dir)
}

pub fn write_voice_audio_file(
    app_namespace: &str,
    message_id: &str,
    opus_blob: &[u8],
) -> Result<String, String> {
    let path = voice_audio_path(app_namespace, message_id)?;
    std::fs::write(&path, opus_blob).map_err(|e| format!("write voice: {e}"))?;
    // Also write a WAV sidecar for Flutter `audioplayers` (raw Opus is not playable there).
    #[cfg(not(target_arch = "wasm32"))]
    {
        if let Ok(pcm) = decode_opus_blob_to_pcm(opus_blob) {
            let wav_path = path.with_extension("wav");
            let _ = write_wav_mono_i16(&wav_path, SAMPLE_RATE_HZ, &pcm);
        }
    }
    Ok(path.to_string_lossy().into_owned())
}

#[cfg(not(target_arch = "wasm32"))]
fn write_wav_mono_i16(path: &std::path::Path, sample_rate: u32, pcm: &[i16]) -> Result<(), String> {
    let data_len = (pcm.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + pcm.len() * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // byte rate
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in pcm {
        out.extend_from_slice(&s.to_le_bytes());
    }
    std::fs::write(path, out).map_err(|e| format!("write wav: {e}"))
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[test]
    fn pcm_roundtrip_opus_blob() {
        let pcm = vec![0i16; FRAME_SAMPLES * 5];
        let blob = encode_pcm_to_opus_blob(&pcm).unwrap();
        assert!(!blob.is_empty());
        let decoded = decode_opus_blob_to_pcm(&blob).unwrap();
        assert_eq!(decoded.len(), FRAME_SAMPLES * 5);
    }

    #[test]
    fn duration_cap_rejects() {
        let err = build_voice_inner(VOICE_MAX_DURATION_MS + 1, &[1, 2, 3]).unwrap_err();
        assert!(err.contains("duration"));
    }

    #[test]
    fn inner_json_roundtrip() {
        let pcm = vec![100i16; FRAME_SAMPLES];
        let blob = encode_pcm_to_opus_blob(&pcm).unwrap();
        let inner = build_voice_inner(20, &blob).unwrap();
        let bytes = inner.to_json_bytes().unwrap();
        let parsed = VoiceInner::from_json_bytes(&bytes).unwrap();
        assert_eq!(parsed.duration_ms, 20);
        assert_eq!(parsed.opus_bytes().unwrap(), blob);
    }
}
