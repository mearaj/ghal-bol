//! Out-of-process native DM node for Linux desktop (survives Flutter UI exit).
//!
//! `cargo run -p ghal_bol --bin ghal_bol_daemon`

use std::env;
use std::process;

use ghal_bol::daemon::{probe_existing_daemon, run_daemon, socket_path_from_env_or_default};

fn main() {
    let socket_path = socket_path_from_env_or_default();
    if env::args().any(|a| a == "--probe") {
        let ok = probe_existing_daemon(&socket_path);
        process::exit(if ok { 0 } else { 1 });
    }
    if probe_existing_daemon(&socket_path) {
        eprintln!(
            "ghal_bol_daemon already running at {}",
            socket_path.display()
        );
        return;
    }
    if let Err(e) = run_daemon(&socket_path) {
        eprintln!("ghal_bol_daemon failed: {e}");
        process::exit(1);
    }
}
