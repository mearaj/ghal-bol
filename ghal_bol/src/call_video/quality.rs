//! Adaptive capture/encode quality for WAN, relay, and cellular paths.
//!
//! Display resolution is handled separately in [`super::render`] (GPU texture shm).
//! This layer scales **encode** resolution and H.264 bitrate from transport pressure.

use super::{RawVideoFrame, i420_downscale_max_edge};

/// Target camera capture (even dimensions). HW/backends may deliver closest size (desktop nokhwa).
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
pub const CAP_WIDTH: u32 = 640;
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
pub const CAP_HEIGHT: u32 = 480;

/// Longest edge written into the cross-process display shm (textures).
/// Match capture resolution — GPU textures scale to full-screen with `BoxFit.contain`.
pub const DISPLAY_MAX_EDGE: u32 = 640;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tier {
    pub max_edge: u32,
    pub bitrate_bps: u32,
}

const TIERS: [Tier; 4] = [
    Tier {
        max_edge: 640,
        bitrate_bps: 2_000_000,
    },
    Tier {
        max_edge: 560,
        bitrate_bps: 1_500_000,
    },
    Tier {
        max_edge: 480,
        bitrate_bps: 1_000_000,
    },
    Tier {
        max_edge: 360,
        bitrate_bps: 500_000,
    },
];

/// Observes wire-queue drops and steps between [`TIERS`].
#[derive(Debug)]
pub struct AdaptiveQuality {
    tier_idx: usize,
    congested_ticks: u8,
    clear_ticks: u8,
    last_logged_tier: usize,
}

impl AdaptiveQuality {
    pub fn new() -> Self {
        Self {
            tier_idx: 0,
            congested_ticks: 0,
            clear_ticks: 0,
            last_logged_tier: 0,
        }
    }

    pub fn tier(&self) -> Tier {
        TIERS[self.tier_idx]
    }

    /// Prepare a frame for the encoder at the current tier.
    pub fn frame_for_encode(&self, frame: &RawVideoFrame) -> RawVideoFrame {
        i420_downscale_max_edge(frame, self.tier().max_edge)
    }

    /// Call once per captured frame with wire chunk send stats.
    pub fn note_wire_send(&mut self, drops: u32, total: u32) {
        // A few `try_send` misses on a 256-deep queue are normal — only treat sustained
        // heavy loss as congestion (avoids false tier steps on LAN).
        let congested = total > 0 && drops * 2 > total;
        if congested {
            self.congested_ticks = self.congested_ticks.saturating_add(1);
            self.clear_ticks = 0;
        } else {
            self.clear_ticks = self.clear_ticks.saturating_add(1);
            if self.clear_ticks >= 6 {
                self.congested_ticks = 0;
            }
        }
    }

    /// Periodic tick (~100 ms). Returns new bitrate when the tier changes.
    pub fn tick(&mut self) -> Option<u32> {
        if self.congested_ticks >= 8 && self.tier_idx + 1 < TIERS.len() {
            self.tier_idx += 1;
            self.congested_ticks = 0;
            self.clear_ticks = 0;
            return Some(self.log_tier_change());
        }
        if self.clear_ticks >= 50 && self.tier_idx > 0 {
            self.tier_idx -= 1;
            self.congested_ticks = 0;
            self.clear_ticks = 0;
            return Some(self.log_tier_change());
        }
        None
    }

    fn log_tier_change(&mut self) -> u32 {
        let t = self.tier();
        if self.last_logged_tier != self.tier_idx {
            self.last_logged_tier = self.tier_idx;
            crate::p2p::native_log::info(
                "call_video",
                format!(
                    "quality_tier max_edge={} bitrate_bps={}",
                    t.max_edge, t.bitrate_bps
                ),
            );
        }
        t.bitrate_bps
    }
}

impl Default for AdaptiveQuality {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_down_on_sustained_drops() {
        let mut q = AdaptiveQuality::new();
        assert_eq!(q.tier().max_edge, 640);
        for _ in 0..8 {
            q.note_wire_send(10, 10);
            let _ = q.tick();
        }
        assert_eq!(q.tier().max_edge, 560);
    }
}
