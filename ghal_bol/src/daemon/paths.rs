//! Default runtime paths for the desktop P2P daemon.

use std::path::PathBuf;

use crate::storage::project_dirs_for_library;

/// `$XDG_RUNTIME_DIR/ghalbol/p2p.sock` when set, else under the app data dir.
pub fn default_socket_path() -> PathBuf {
    if let Ok(runtime) = std::env::var("XDG_RUNTIME_DIR") {
        if !runtime.is_empty() {
            return PathBuf::from(runtime).join("ghalbol").join("p2p.sock");
        }
    }
    let mut p = project_dirs_for_library()
        .map(|d| d.data_local_dir().to_path_buf())
        .unwrap_or_else(|_| PathBuf::from("/tmp"));
    p.push("ghalbol");
    p.push("p2p.sock");
    p
}
