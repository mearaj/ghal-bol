//! Shared helpers for libp2p integration tests (separate test binaries).
//!
//! Sets [`GHAL_BOL_MINIMAL_SWARM`] (TCP-only transport, no QUIC/DNS) so `build_swarm` fits GitHub
//! runner thread stacks (~8 MiB).

/// Call at the start of each integration test (before spawning P2P threads).
pub fn init_integration_env() {
    // SAFETY: single-threaded test harness; no concurrent env access.
    unsafe { std::env::set_var("GHAL_BOL_MINIMAL_SWARM", "1") };
}

/// Parent std thread stack (libp2p tests spawn a dedicated thread per node).
pub const P2P_TEST_THREAD_STACK: usize = 16 * 1024 * 1024;

/// Tokio current-thread / blocking-pool stack (coord relay fetch, test runtimes).
pub const P2P_TOKIO_BLOCKING_STACK: usize = 16 * 1024 * 1024;

pub fn spawn_p2p_thread<F>(name: &str, f: F) -> std::thread::JoinHandle<()>
where
    F: FnOnce() + Send + 'static,
{
    std::thread::Builder::new()
        .name(name.to_string())
        .stack_size(P2P_TEST_THREAD_STACK)
        .spawn(f)
        .unwrap_or_else(|e| panic!("spawn p2p test thread {name}: {e}"))
}

/// Single-thread runtime: gossip futures hold `mpsc::Receiver` (not `Send`), so they
/// must poll on the parent thread that already carries `P2P_TEST_THREAD_STACK`.
pub fn p2p_tokio_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .thread_stack_size(P2P_TOKIO_BLOCKING_STACK)
        .build()
        .unwrap_or_else(|e| panic!("p2p tokio runtime: {e}"))
}

pub fn block_on_local<F, T>(f: F) -> T
where
    F: std::future::Future<Output = T>,
{
    let rt = p2p_tokio_runtime();
    rt.block_on(f)
}
