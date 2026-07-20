//! Cross-process GPU texture backing store.
//!
//! The video engine (daemon / `:p2p`) writes the latest display RGBA into a mmap
//! file; the Flutter embedder reads it into a platform `Texture` without JSON,
//! base64, or Dart pixel work. See `docs/GHAL_BOL_VIDEO_NATIVE_V1.md`.

use super::{RawVideoFrame, i420_to_rgba_max_edge};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex as StdMutex, OnceLock};

use super::quality::DISPLAY_MAX_EDGE;

const MAGIC: &[u8; 4] = b"GBV1";
/// Fixed header before RGBA payload.
pub const SHM_HEADER_SIZE: usize = 32;

fn video_shm_root() -> PathBuf {
    #[cfg(test)]
    {
        if let Ok(g) = test_shm_root().lock() {
            if let Some(p) = g.as_ref() {
                return p.clone();
            }
        }
    }
    #[cfg(target_os = "android")]
    {
        if let Some(dir) = crate::c_ffi::optional_android_data_dir() {
            return dir.join("video_shm");
        }
        // `:p2p` calls configureDataDirectory before video; this is a pre-start fallback only.
        return std::env::temp_dir().join("ghal_bol_video");
    }
    #[cfg(target_os = "linux")]
    {
        return PathBuf::from("/dev/shm/ghal_bol_video");
    }
    #[cfg(not(any(target_os = "android", target_os = "linux")))]
    {
        std::env::temp_dir().join("ghal_bol_video")
    }
}

#[cfg(test)]
fn test_shm_root() -> &'static StdMutex<Option<PathBuf>> {
    static REG: OnceLock<StdMutex<Option<PathBuf>>> = OnceLock::new();
    REG.get_or_init(|| StdMutex::new(None))
}

#[cfg(test)]
pub(crate) fn set_test_shm_root(path: PathBuf) {
    if let Ok(mut g) = test_shm_root().lock() {
        *g = Some(path);
    }
}

fn sanitize_token(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Absolute path to the mmap file for `(call_id, track)`.
pub fn shm_path(call_id: &str, track: &str) -> PathBuf {
    let root = video_shm_root();
    let name = format!("{}_{}.rgba", sanitize_token(call_id), sanitize_token(track));
    root.join(name)
}

fn ensure_shm_dir() {
    let root = video_shm_root();
    let _ = fs::create_dir_all(&root);
}

fn write_header(buf: &mut [u8], w: u32, h: u32, generation: u64) {
    buf[0..4].copy_from_slice(MAGIC);
    buf[4..8].copy_from_slice(&w.to_le_bytes());
    buf[8..12].copy_from_slice(&h.to_le_bytes());
    buf[12..20].copy_from_slice(&generation.to_le_bytes());
    buf[20..32].fill(0);
}

/// Publish a display frame into the cross-process shm file.
pub fn publish_display_frame(call_id: &str, track: &str, frame: &RawVideoFrame, generation: u64) {
    let (rgba, w, h) = i420_to_rgba_max_edge(frame, DISPLAY_MAX_EDGE);
    if w == 0 || h == 0 || rgba.is_empty() {
        return;
    }
    ensure_shm_dir();
    let path = shm_path(call_id, track);
    let mut out = Vec::with_capacity(SHM_HEADER_SIZE + rgba.len());
    out.resize(SHM_HEADER_SIZE, 0);
    out.extend_from_slice(&rgba);
    // Header last in the vec, but we write payload first in the file layout; assemble
    // full file then replace header so readers never see a bumped generation with stale pixels.
    write_header(&mut out[0..SHM_HEADER_SIZE], w, h, generation);
    let _ = fs::write(path, out);
}

/// Metadata for the Flutter embedder to register a `Texture`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextureShmInfo {
    pub path: String,
    pub width: u32,
    pub height: u32,
    pub generation: u64,
}

/// Read shm header without mapping the full RGBA payload.
pub fn texture_shm_info(call_id: &str, track: &str) -> Option<TextureShmInfo> {
    let path = shm_path(call_id, track);
    let data = fs::read(&path).ok()?;
    if data.len() < SHM_HEADER_SIZE {
        return None;
    }
    if &data[0..4] != MAGIC {
        return None;
    }
    let w = u32::from_le_bytes(data[4..8].try_into().ok()?);
    let h = u32::from_le_bytes(data[8..12].try_into().ok()?);
    let generation = u64::from_le_bytes(data[12..20].try_into().ok()?);
    if w == 0 || h == 0 {
        return None;
    }
    let expected = SHM_HEADER_SIZE + (w as usize) * (h as usize) * 4;
    if data.len() < expected {
        return None;
    }
    Some(TextureShmInfo {
        path: path.to_string_lossy().into_owned(),
        width: w,
        height: h,
        generation,
    })
}

fn tracked_calls() -> &'static StdMutex<HashMap<String, u32>> {
    static REG: OnceLock<StdMutex<HashMap<String, u32>>> = OnceLock::new();
    REG.get_or_init(|| StdMutex::new(HashMap::new()))
}

/// Remember active calls so `release_call` can delete shm files.
pub fn track_call(call_id: &str) {
    if let Ok(mut m) = tracked_calls().lock() {
        *m.entry(call_id.to_string()).or_insert(0) += 1;
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shm_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        set_test_shm_root(dir.path().to_path_buf());
        let call_id = "test-call-1";
        let track = "remote";
        let frame = RawVideoFrame {
            width: 4,
            height: 4,
            data: vec![128u8; 4 * 4 + 2 * 2 * 2],
        };
        publish_display_frame(call_id, track, &frame, 1);
        let info = texture_shm_info(call_id, track);
        assert!(info.is_some());
        let info = info.unwrap();
        assert!(info.generation == 1);
        assert!(info.width > 0 && info.height > 0);
    }
}
