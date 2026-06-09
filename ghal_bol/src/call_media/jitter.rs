//! Adaptive jitter buffer: reorders out-of-order media frames, drops stale ones,
//! and signals gaps so the decoder can run packet-loss concealment (PLC).

use std::collections::BTreeMap;

use super::MediaFrame;

/// What the playout tick should do for this 20 ms slot.
pub enum Playout {
    /// A real frame is ready to decode.
    Frame(MediaFrame),
    /// The next frame is genuinely missing (later frames exist) → run PLC.
    Conceal,
    /// Not enough buffered yet, or underrun → play silence, do not advance.
    Silence,
}

pub struct JitterBuffer {
    /// Frames buffered before playout starts (prebuffer depth).
    target_depth: usize,
    /// Hard cap; beyond this we drop oldest to bound latency.
    max_depth: usize,
    map: BTreeMap<u64, MediaFrame>,
    next_seq: Option<u64>,
    started: bool,
}

impl JitterBuffer {
    pub fn new(target_depth: usize, max_depth: usize) -> Self {
        Self {
            target_depth: target_depth.max(1),
            max_depth: max_depth.max(target_depth.max(1)),
            map: BTreeMap::new(),
            next_seq: None,
            started: false,
        }
    }

    /// Insert a received frame. Frames older than the play cursor are dropped.
    pub fn push(&mut self, frame: MediaFrame) {
        if let Some(n) = self.next_seq {
            if frame.seq < n {
                return; // too late — already played past this
            }
        }
        self.map.insert(frame.seq, frame);
        while self.map.len() > self.max_depth {
            let oldest = match self.map.keys().next().copied() {
                Some(k) => k,
                None => break,
            };
            self.map.remove(&oldest);
            if let Some(n) = self.next_seq {
                if oldest >= n {
                    self.next_seq = Some(oldest + 1);
                }
            }
        }
    }

    /// Produce the next playout decision (call once per 20 ms).
    pub fn pop(&mut self) -> Playout {
        if !self.started {
            if self.map.len() < self.target_depth {
                return Playout::Silence;
            }
            self.started = true;
            self.next_seq = self.map.keys().next().copied();
        }
        let n = match self.next_seq {
            Some(n) => n,
            None => return Playout::Silence,
        };
        if let Some(frame) = self.map.remove(&n) {
            self.next_seq = Some(n + 1);
            return Playout::Frame(frame);
        }
        // n is missing: is there anything newer? then it's a real loss.
        if self.map.range((n + 1)..).next().is_some() {
            self.next_seq = Some(n + 1);
            return Playout::Conceal;
        }
        // Underrun: nothing to play and nothing ahead → re-prebuffer.
        self.started = false;
        Playout::Silence
    }
}
