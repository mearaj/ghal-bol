//! Ghal Bol coordination server binary.
//!
//! Presence and endpoint discovery only — no chat transcripts or message payloads.

use ghal_bol_server::{app, relay, AppState, RelayConfig, ServerConfig};
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let config = ServerConfig::from_env();
    // Bind coord HTTP first — if the port is taken, fail before starting the libp2p relay
    // (otherwise logs show "relay started" then AddrInUse, which looks like a relay bug).
    let listener = TcpListener::bind(config.listen).await?;

    let state = Arc::new(AppState::open(config.clone())?);

    // Co-located Circuit Relay v2 node (NAT traversal). The HTTP API stays a lightweight
    // phone book; the relay only carries brief NAT-traversal traffic until DCUtR upgrades
    // clients to a direct connection. Advertised to clients at GET /v1/relay.
    let relay_data_dir = config
        .database_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    match relay::start(RelayConfig::from_env(&relay_data_dir)) {
        Ok(Some(info)) => state.set_relay_info(info),
        Ok(None) => {}
        Err(e) => tracing::warn!(error = %e, "relay node failed to start — continuing HTTP only"),
    }

    let shutdown = Arc::new(Notify::new());
    let purge = spawn_purge_task(Arc::clone(&state), Arc::clone(&shutdown));
    let app = app(state);

    // Dual-stack: also serve the counterpart IP family on the same port so both IPv4 and IPv6
    // clients can reach coord (IPv6 is preferred when both work). Best-effort — a host without an
    // IPv6 (or IPv4) stack simply keeps the primary listener.
    let counterpart = bind_counterpart_listener(config.listen).await;

    tracing::info!(
        listen = %config.listen,
        dual_stack = counterpart.is_some(),
        db = %config.database_path.display(),
        "ghal_bol_server listening (ctrl+c to stop)"
    );

    // Trigger graceful shutdown for every server task on the first signal.
    {
        let shutdown_notify = Arc::clone(&shutdown);
        tokio::spawn(async move {
            shutdown_signal().await;
            shutdown_notify.notify_waiters();
        });
    }

    let mut servers: Vec<JoinHandle<()>> =
        vec![spawn_http_server(listener, app.clone(), Arc::clone(&shutdown))];
    if let Some(listener6) = counterpart {
        servers.push(spawn_http_server(listener6, app, Arc::clone(&shutdown)));
    }
    for s in servers {
        let _ = s.await;
    }

    purge.abort();
    tracing::info!("ghal_bol_server stopped");
    Ok(())
}

/// Serve the router on a listener until the shared shutdown is notified.
fn spawn_http_server(
    listener: TcpListener,
    app: axum::Router,
    shutdown: Arc<Notify>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let serve = axum::serve(listener, app).with_graceful_shutdown(async move {
            shutdown.notified().await;
            // Do not block Ctrl+C forever on slow/stuck HTTP clients.
            tokio::time::sleep(Duration::from_secs(3)).await;
        });
        if let Err(e) = serve.await {
            tracing::warn!(error = %e, "http server task ended with error");
        }
    })
}

/// Map a listen address to its counterpart-family address **preserving scope** so the server is
/// reachable over both IP families without changing exposure: a wildcard maps to the other-family
/// wildcard, loopback to other-family loopback. A specific interface IP has no safe counterpart
/// (returns `None`) — we must never widen a loopback/single-IP bind into a public wildcard.
fn counterpart_addr(primary: SocketAddr) -> Option<SocketAddr> {
    use std::net::{Ipv4Addr, Ipv6Addr};
    let port = primary.port();
    match primary.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => {
            Some(SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), port))
        }
        IpAddr::V4(ip) if ip.is_loopback() => {
            Some(SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), port))
        }
        IpAddr::V6(ip) if ip.is_unspecified() => {
            Some(SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port))
        }
        IpAddr::V6(ip) if ip.is_loopback() => {
            Some(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port))
        }
        _ => None,
    }
}

/// Bind a listener for the IP family not covered by `primary` (same port, same scope), so the
/// server is reachable over both IPv4 and IPv6. The IPv6 socket is forced `V6ONLY` to avoid
/// clashing with an IPv4 wildcard already bound. Returns `None` (with a log) when there is no safe
/// counterpart (specific-IP bind) or the counterpart stack is unavailable — single-stack continues.
async fn bind_counterpart_listener(primary: SocketAddr) -> Option<TcpListener> {
    use socket2::{Domain, Protocol, Socket, Type};

    let Some(addr) = counterpart_addr(primary) else {
        tracing::debug!(
            listen = %primary,
            "coord HTTP bound to a specific IP — no dual-stack counterpart"
        );
        return None;
    };
    let domain = match addr.ip() {
        IpAddr::V6(_) => Domain::IPV6,
        IpAddr::V4(_) => Domain::IPV4,
    };

    let build = || -> std::io::Result<TcpListener> {
        let socket = Socket::new(domain, Type::STREAM, Some(Protocol::TCP))?;
        if domain == Domain::IPV6 {
            socket.set_only_v6(true)?;
        }
        socket.set_reuse_address(true)?;
        socket.set_nonblocking(true)?;
        socket.bind(&addr.into())?;
        socket.listen(1024)?;
        TcpListener::from_std(std::net::TcpListener::from(socket))
    };

    match build() {
        Ok(l) => {
            tracing::info!(listen = %addr, "coord HTTP also listening (dual-stack)");
            Some(l)
        }
        Err(e) => {
            tracing::warn!(
                listen = %addr,
                error = %e,
                "coord HTTP counterpart-family bind failed — continuing single-stack"
            );
            None
        }
    }
}

fn spawn_purge_task(state: Arc<AppState>, shutdown: Arc<Notify>) -> JoinHandle<()> {
    let presence = Arc::clone(&state.presence);
    let ttl = state.config.presence_ttl;
    let every = state.config.purge_interval;
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(every);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        'purge: loop {
            tokio::select! {
                _ = shutdown.notified() => {
                    tracing::debug!("purge task stopped");
                    break 'purge;
                }
                _ = tick.tick() => {
                    let store = Arc::clone(&presence);
                    let t = ttl;
                    let stop = Arc::clone(&shutdown);
                    let purge = tokio::task::spawn_blocking(move || store.purge_expired(t));
                    tokio::select! {
                        _ = stop.notified() => break 'purge,
                        r = purge => match r {
                            Ok(Ok(n)) if n > 0 => {
                                tracing::info!(removed = n, "purged expired peers")
                            }
                            Ok(Err(e)) => tracing::warn!(error = %e, "purge failed"),
                            Err(e) => tracing::warn!(error = %e, "purge task join failed"),
                            _ => {}
                        },
                    }
                }
            }
        }
    })
}

async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();
    #[cfg(unix)]
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("failed to install SIGTERM handler");

    tokio::select! {
        _ = ctrl_c => {
            tracing::info!("Ctrl+C received — shutting down");
        }
        _ = async {
            #[cfg(unix)]
            {
                terminate.recv().await;
            }
            #[cfg(not(unix))]
            {
                std::future::pending::<()>().await;
            }
        } => {
            tracing::info!("SIGTERM received — shutting down");
        }
    }

    // Second Ctrl+C: exit immediately (graceful drain already started).
    tokio::spawn(async {
        if tokio::signal::ctrl_c().await.is_ok() {
            tracing::warn!("second Ctrl+C — exiting now");
            std::process::exit(130);
        }
    });
}
