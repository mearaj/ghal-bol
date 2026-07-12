//! Cross-cutting DM / store diagnostics → Flutter App log (via P2P `native_log` sink when running).

use std::fmt::Display;

#[cfg(not(target_arch = "wasm32"))]
pub fn info(tag: &str, message: impl Display) {
    crate::p2p::native_log::info(tag, message);
}

#[cfg(not(target_arch = "wasm32"))]
pub fn warn(tag: &str, message: impl Display) {
    crate::p2p::native_log::warn(tag, message);
}

#[cfg(not(target_arch = "wasm32"))]
pub fn debug(tag: &str, message: impl Display) {
    crate::p2p::native_log::debug(tag, message);
}

#[cfg(target_arch = "wasm32")]
pub fn info(_tag: &str, _message: impl Display) {}

#[cfg(target_arch = "wasm32")]
pub fn warn(_tag: &str, _message: impl Display) {}

#[cfg(target_arch = "wasm32")]
pub fn debug(_tag: &str, _message: impl Display) {}

/// Shorten hex for log lines (first 8 + last 8).
pub fn short_hex(hex: &str) -> String {
    let s = hex.trim();
    if s.len() <= 20 {
        return s.to_string();
    }
    format!("{}…{}", &s[..8], &s[s.len() - 8..])
}
