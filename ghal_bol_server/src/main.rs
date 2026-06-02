//! Ghal Bol coordination server binary.
//!
//! Presence and endpoint discovery only — no chat transcripts or message payloads.

use ghal_bol_server::{app, AppState, ServerConfig};
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
    let state = Arc::new(AppState::open(config.clone())?);
    let shutdown = Arc::new(Notify::new());
    let purge = spawn_purge_task(Arc::clone(&state), Arc::clone(&shutdown));
    let app = app(state);

    let listener = TcpListener::bind(config.listen).await?;
    tracing::info!(
        listen = %config.listen,
        db = %config.database_path.display(),
        "ghal_bol_server listening (ctrl+c to stop)"
    );

    let shutdown_notify = Arc::clone(&shutdown);
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            shutdown_notify.notify_waiters();
            // Do not block Ctrl+C forever on slow/stuck HTTP clients.
            tokio::time::sleep(Duration::from_secs(3)).await;
        })
        .await?;

    purge.abort();
    tracing::info!("ghal_bol_server stopped");
    Ok(())
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
