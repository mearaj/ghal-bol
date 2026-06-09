//! Unix-socket JSON-RPC for the out-of-process **`ghal_bol_daemon`** (Linux desktop).

mod paths;
mod server;

pub use paths::{default_socket_path, touch_incoming_call_wake};
pub use server::{probe_existing_daemon, run_daemon, socket_path_from_env_or_default};
