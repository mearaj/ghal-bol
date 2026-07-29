//! Length-prefixed channel mux inside a Noise transport session.

pub const MUX_HEADER_LEN: usize = 8;
pub const CHANNEL_MSG: u32 = 0;
pub const CHANNEL_CALL_AUDIO: u32 = 1;
pub const CHANNEL_CALL_VIDEO: u32 = 2;
pub const CHANNEL_ATTACH: u32 = 3;
pub const CHANNEL_KEEPALIVE: u32 = 0xFFFF_FFFF;
