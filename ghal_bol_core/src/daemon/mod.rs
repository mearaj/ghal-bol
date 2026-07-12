//! Unix-socket JSON-RPC for the out-of-process **`ghal_bol_core_daemon`** (Linux desktop).

mod client;
mod client_api;
mod integrator_config;
mod paths;
mod server;
mod ui_session;

pub use client::{DaemonClient, RpcError};
pub use client_api::{
    DaemonMethod, DaemonPollEventKind, UiIntegratorCallbacks, UiWakeKind, dispatch_method,
};
pub use integrator_config::IntegratorConfig;
pub use paths::{
    clear_incoming_call_wake, clear_unlock_wake, default_socket_path,
    default_socket_path_for_app_namespace, incoming_call_wake_path, runtime_dir_for_app_namespace,
    sanitize_app_namespace_segment, take_incoming_call_wake, take_unlock_wake,
    touch_incoming_call_wake, touch_unlock_wake, ui_presence_active, ui_presence_path,
    unlock_wake_path,
};
pub use server::{probe_existing_daemon, run_daemon, socket_path_from_env_or_default};
pub use ui_session::{
    UiSessionGuard, suppress_ui_exit_hangup_ms, ui_process_exiting, ui_session_active,
};
