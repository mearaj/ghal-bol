//! Video frame fragmentation + reassembly.
//!
//! A coded video frame is far larger than one media packet (and far larger than a
//! UDP datagram for the future QUIC-datagram transport), so each frame is split
//! into ordered chunks. Each chunk becomes one sealed media packet on the wire.
//! The receiver reassembles chunks by `frame_seq` until `chunk_cnt` are present.
//!
//! Chunk payload (the plaintext that `call_media::MediaCrypto` seals):
//! `frame_seq(4 LE) | chunk_idx(2 LE) | chunk_cnt(2 LE) | flags(1) | codec_bytes`

use std::collections::{BTreeMap, HashMap};

use super::codec::EncodedVideoFrame;

pub const VIDEO_CHUNK_HEADER_LEN: usize = 4 + 2 + 2 + 1;
const FLAG_KEYFRAME: u8 = 0x01;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VideoChunk {
    pub frame_seq: u32,
    pub chunk_idx: u16,
    pub chunk_cnt: u16,
    pub keyframe: bool,
    pub data: Vec<u8>,
}

impl VideoChunk {
    pub fn to_payload(&self) -> Vec<u8> {
        let mut p = Vec::with_capacity(VIDEO_CHUNK_HEADER_LEN + self.data.len());
        p.extend_from_slice(&self.frame_seq.to_le_bytes());
        p.extend_from_slice(&self.chunk_idx.to_le_bytes());
        p.extend_from_slice(&self.chunk_cnt.to_le_bytes());
        p.push(if self.keyframe { FLAG_KEYFRAME } else { 0 });
        p.extend_from_slice(&self.data);
        p
    }

    pub fn from_payload(payload: &[u8]) -> Result<VideoChunk, String> {
        if payload.len() < VIDEO_CHUNK_HEADER_LEN {
            return Err("video chunk payload too short".to_string());
        }
        let frame_seq = u32::from_le_bytes(payload[0..4].try_into().unwrap());
        let chunk_idx = u16::from_le_bytes(payload[4..6].try_into().unwrap());
        let chunk_cnt = u16::from_le_bytes(payload[6..8].try_into().unwrap());
        let keyframe = payload[8] & FLAG_KEYFRAME != 0;
        if chunk_cnt == 0 || chunk_idx >= chunk_cnt {
            return Err("video chunk indices invalid".to_string());
        }
        let data = payload[VIDEO_CHUNK_HEADER_LEN..].to_vec();
        Ok(VideoChunk { frame_seq, chunk_idx, chunk_cnt, keyframe, data })
    }
}

/// Split one encoded frame into ordered chunks of at most `chunk_data_bytes` codec
/// bytes each. An empty frame still yields one (empty) chunk so it is delivered.
pub fn fragment_frame(
    frame_seq: u32,
    enc: &EncodedVideoFrame,
    chunk_data_bytes: usize,
) -> Vec<VideoChunk> {
    let cdb = chunk_data_bytes.max(1);
    if enc.data.is_empty() {
        return vec![VideoChunk {
            frame_seq,
            chunk_idx: 0,
            chunk_cnt: 1,
            keyframe: enc.keyframe,
            data: Vec::new(),
        }];
    }
    let total = enc.data.len().div_ceil(cdb);
    let chunk_cnt = total.min(u16::MAX as usize) as u16;
    let mut chunks = Vec::with_capacity(total);
    for (i, slice) in enc.data.chunks(cdb).enumerate() {
        chunks.push(VideoChunk {
            frame_seq,
            chunk_idx: i as u16,
            chunk_cnt,
            keyframe: enc.keyframe,
            data: slice.to_vec(),
        });
    }
    chunks
}

struct Partial {
    chunk_cnt: u16,
    keyframe: bool,
    chunks: BTreeMap<u16, Vec<u8>>,
}

/// Reassembles chunks into complete encoded frames. Bounds memory by capping the
/// number of in-flight partial frames (drops the oldest partial on overflow — a
/// frame that lost a chunk and will never complete).
pub struct Reassembler {
    pending: HashMap<u32, Partial>,
    max_pending: usize,
}

impl Reassembler {
    pub fn new(max_pending: usize) -> Self {
        Self { pending: HashMap::new(), max_pending: max_pending.max(2) }
    }

    /// Push a received chunk. Returns the completed `(frame_seq, frame)` when the
    /// last missing chunk of a frame arrives.
    pub fn push(&mut self, c: VideoChunk) -> Option<(u32, EncodedVideoFrame)> {
        let fs = c.frame_seq;
        let entry = self.pending.entry(fs).or_insert_with(|| Partial {
            chunk_cnt: c.chunk_cnt,
            keyframe: false,
            chunks: BTreeMap::new(),
        });
        entry.keyframe |= c.keyframe;
        entry.chunks.insert(c.chunk_idx, c.data);
        if entry.chunks.len() as u16 >= entry.chunk_cnt {
            let p = self.pending.remove(&fs).unwrap();
            let mut data = Vec::new();
            for (_, d) in p.chunks {
                data.extend_from_slice(&d);
            }
            return Some((fs, EncodedVideoFrame { keyframe: p.keyframe, data }));
        }
        while self.pending.len() > self.max_pending {
            let oldest = self.pending.keys().min().copied();
            match oldest {
                Some(k) => {
                    self.pending.remove(&k);
                }
                None => break,
            }
        }
        None
    }
}
