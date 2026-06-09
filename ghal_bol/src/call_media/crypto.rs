//! Per-frame AES-256-GCM seal/unseal for call media, keyed by the identity media
//! key (`call_media_key::derive_call_media_keys_from_identity`).
//!
//! Both peers share the **same** `frame_key`, so a per-direction byte is mixed
//! into the nonce to guarantee nonce uniqueness across the two senders (GCM
//! nonce reuse under one key is catastrophic). The sender counter is monotonic
//! per direction; together `(dir, counter)` is a unique nonce per (key, packet).
//!
//! Wire packet: `dir(1) || counter(8 LE) || GCM(ts(4) || flags(1) || payload)`.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};

use super::MediaFrame;

pub struct MediaCrypto {
    cipher: Aes256Gcm,
    /// 0 when local identity sorts lower than peer, else 1 — opposite on each side.
    dir: u8,
}

impl MediaCrypto {
    pub fn new(frame_key: &[u8; 32], local_is_a: bool) -> Self {
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(frame_key));
        Self { cipher, dir: if local_is_a { 0 } else { 1 } }
    }

    fn nonce(dir: u8, counter: u64) -> [u8; 12] {
        let mut n = [0u8; 12];
        n[0] = dir;
        n[4..12].copy_from_slice(&counter.to_le_bytes());
        n
    }

    /// Seal a frame for transmission. Uses our [`dir`] + `frame.seq` as the nonce.
    pub fn seal(&self, frame: &MediaFrame) -> Result<Vec<u8>, String> {
        let mut pt = Vec::with_capacity(5 + frame.payload.len());
        pt.extend_from_slice(&frame.ts.to_le_bytes());
        pt.push(frame.flags);
        pt.extend_from_slice(&frame.payload);

        let nonce = Self::nonce(self.dir, frame.seq);
        let ct = self
            .cipher
            .encrypt(Nonce::from_slice(&nonce), pt.as_ref())
            .map_err(|_| "media seal failed".to_string())?;

        let mut wire = Vec::with_capacity(9 + ct.len());
        wire.push(self.dir);
        wire.extend_from_slice(&frame.seq.to_le_bytes());
        wire.extend_from_slice(&ct);
        Ok(wire)
    }

    /// Open a received wire packet. The nonce direction comes from the wire (the
    /// peer's direction), so the same shared key decrypts the opposite stream.
    pub fn open(&self, wire: &[u8]) -> Result<MediaFrame, String> {
        if wire.len() < 9 {
            return Err("media wire too short".to_string());
        }
        let dir = wire[0];
        let counter = u64::from_le_bytes(wire[1..9].try_into().unwrap());
        let nonce = Self::nonce(dir, counter);
        let pt = self
            .cipher
            .decrypt(Nonce::from_slice(&nonce), &wire[9..])
            .map_err(|_| "media open failed".to_string())?;
        if pt.len() < 5 {
            return Err("media plaintext too short".to_string());
        }
        let ts = u32::from_le_bytes(pt[0..4].try_into().unwrap());
        let flags = pt[4];
        let payload = pt[5..].to_vec();
        Ok(MediaFrame { seq: counter, ts, flags, payload })
    }
}
