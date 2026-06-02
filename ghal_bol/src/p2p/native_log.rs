//! In-process diagnostic log for the DM node (forwarded to Flutter via `GossipChatEvent::NativeLog`).
//!
//! - **stderr / adb logcat:** all levels (grep `ghal_bol/`).
//! - **Flutter App log:** `info`/`warn`/`error` for connectivity tags; `debug` only when
//!   `GHAL_BOL_VERBOSE_LOG=1` (avoids UI stalls from kad/mDNS tick noise).

use std::fmt::Display;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

#[derive(Clone, Debug)]
pub struct NativeLogLine {
    pub level: &'static str,
    pub tag: String,
    pub message: String,
}

type LogSink = Box<dyn Fn(NativeLogLine) + Send + Sync>;

static SINK: OnceLock<Mutex<Option<LogSink>>> = OnceLock::new();
static VERBOSE: OnceLock<AtomicBool> = OnceLock::new();

fn sink_mx() -> &'static Mutex<Option<LogSink>> {
    SINK.get_or_init(|| Mutex::new(None))
}

fn verbose_mx() -> &'static AtomicBool {
    VERBOSE.get_or_init(|| {
        let on = std::env::var("GHAL_BOL_VERBOSE_LOG")
            .map(|v| {
                let v = v.trim().to_ascii_lowercase();
                matches!(v.as_str(), "1" | "true" | "yes" | "on")
            })
            .unwrap_or(false);
        AtomicBool::new(on)
    })
}

/// High-volume libp2p detail (`debug` native lines forwarded to Flutter when true).
pub fn verbose_enabled() -> bool {
    verbose_mx().load(Ordering::Relaxed)
}

/// Register FFI/event forwarder while the P2P worker runs. Pass `None` on stop.
pub fn set_sink(sink: Option<LogSink>) {
    if let Ok(mut g) = sink_mx().lock() {
        *g = sink;
    }
}

fn stderr_line(level: &str, tag: &str, message: &str) {
    eprintln!("[ghal_bol/{level}] [{tag}] {message}");
}

fn emit(level: &'static str, tag: &str, message: impl Display) {
    let msg = message.to_string();
    // Always mirror to stderr/logcat/journald so `adb logcat | grep ghal_bol` shows full libp2p flow.
    stderr_line(level, tag, &msg);

    if level == "debug" && !verbose_enabled() {
        return;
    }

    let line = NativeLogLine {
        level,
        tag: tag.to_string(),
        message: msg,
    };
    if let Ok(g) = sink_mx().lock() {
        if let Some(s) = g.as_ref() {
            s(line);
        }
    }
}

pub fn debug(tag: &str, message: impl Display) {
    emit("debug", tag, message);
}

pub fn info(tag: &str, message: impl Display) {
    emit("info", tag, message);
}

pub fn warn(tag: &str, message: impl Display) {
    emit("warn", tag, message);
}

pub fn error(tag: &str, message: impl Display) {
    emit("error", tag, message);
}
