//! Ghal Bol coordination server binary.
//!
//! Presence and endpoint discovery only — no chat transcripts or message payloads.

use ghal_bol_coord::{AppState, DdnsConfig, ServerConfig, app, spawn_ddns_task};
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
    let listener = TcpListener::bind(config.listen).await?;
    let state = Arc::new(AppState::open(config.clone())?);

    let shutdown = Arc::new(Notify::new());
    let purge = spawn_purge_task(Arc::clone(&state), Arc::clone(&shutdown));
    let bridge_sweep = spawn_bridge_sweep_task(Arc::clone(&state), Arc::clone(&shutdown));
    let ddns = DdnsConfig::from_env().map(|cfg| spawn_ddns_task(cfg, Arc::clone(&shutdown)));
    let app = app(state);

    let counterpart = bind_counterpart_listener(config.listen).await;

    tracing::info!(
        listen = %config.listen,
        dual_stack = counterpart.is_some(),
        db = %config.database_path.display(),
        "ghal_bol_coord listening (ctrl+c to stop)"
    );

    {
        let shutdown_notify = Arc::clone(&shutdown);
        tokio::spawn(async move {
            shutdown_signal().await;
            shutdown_notify.notify_waiters();
        });
    }

    let mut servers: Vec<JoinHandle<()>> = vec![spawn_http_server(
        listener,
        app.clone(),
        Arc::clone(&shutdown),
    )];
    if let Some(listener6) = counterpart {
        servers.push(spawn_http_server(listener6, app, Arc::clone(&shutdown)));
    }
    for s in servers {
        let _ = s.await;
    }

    purge.abort();
    bridge_sweep.abort();
    if let Some(handle) = ddns {
        handle.abort();
    }
    tracing::info!("ghal_bol_coord stopped");
    Ok(())
}

fn spawn_http_server(
    listener: TcpListener,
    app: axum::Router,
    shutdown: Arc<Notify>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let serve = axum::serve(listener, app).with_graceful_shutdown(async move {
            shutdown.notified().await;
            tokio::time::sleep(Duration::from_secs(3)).await;
        });
        if let Err(e) = serve.await {
            tracing::warn!(error = %e, "http server task ended with error");
        }
    })
}

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

async fn bind_counterpart_listener(primary: SocketAddr) -> Option<TcpListener> {
    use socket2::{Domain, Protocol, Socket, Type};

    let Some(addr) = counterpart_addr(primary) else {
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
                _ = shutdown.notified() => break 'purge,
                _ = tick.tick() => {
                    let store = Arc::clone(&presence);
                    let t = ttl;
                    let stop = Arc::clone(&shutdown);
                    let purge = tokio::task::spawn_blocking(move || store.purge_expired(t));
                    tokio::select! {
                        _ = stop.notified() => break 'purge,
                        r = purge => match r {
                            Ok(Ok(n)) if n > 0 => tracing::info!(removed = n, "purged expired peers"),
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

fn spawn_bridge_sweep_task(state: Arc<AppState>, shutdown: Arc<Notify>) -> JoinHandle<()> {
    let bridge = state.bridge.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(30));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = shutdown.notified() => break,
                _ = tick.tick() => bridge.purge_expired().await,
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
        _ = ctrl_c => tracing::info!("Ctrl+C received — shutting down"),
        _ = async {
            #[cfg(unix)]
            { terminate.recv().await; }
            #[cfg(not(unix))]
            { std::future::pending::<()>().await; }
        } => tracing::info!("SIGTERM received — shutting down"),
    }

    tokio::spawn(async {
        if tokio::signal::ctrl_c().await.is_ok() {
            tracing::warn!("second Ctrl+C — exiting now");
            std::process::exit(130);
        }
    });
}
