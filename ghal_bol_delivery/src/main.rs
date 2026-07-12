//! Ghal Bol delivery server binary.

use axum_server::tls_rustls::RustlsConfig;
use ghal_bol_delivery::{
    AppState, DeliveryConfig, app, export_mailbox, import_mailbox, mailbox_stats,
};
use ghal_bol_ddns::{DdnsConfig, spawn_ddns_task};
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::Notify;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(|s| s.as_str()) {
        Some("export-mailbox") => return run_export_mailbox(&args),
        Some("import-mailbox") => return run_import_mailbox(&args),
        Some("mailbox-stats") => return run_mailbox_stats(),
        _ => run_server().await,
    }
}

fn run_export_mailbox(args: &[String]) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let out = parse_flag(args, "--out").ok_or("--out required")?;
    let config = DeliveryConfig::from_env();
    export_mailbox(&config, std::path::Path::new(&out))?;
    println!("exported mailbox to {out}");
    Ok(())
}

fn run_import_mailbox(args: &[String]) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let input = parse_flag(args, "--in").ok_or("--in required")?;
    let replace = args.iter().any(|a| a == "--replace");
    let config = DeliveryConfig::from_env();
    let stats = import_mailbox(&config, std::path::Path::new(&input), replace)?;
    println!("{}", serde_json::to_string_pretty(&stats)?);
    Ok(())
}

fn run_mailbox_stats() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let config = DeliveryConfig::from_env();
    let stats = mailbox_stats(&config)?;
    println!("{}", serde_json::to_string_pretty(&stats)?);
    Ok(())
}

fn parse_flag(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
        .filter(|s| !s.is_empty())
}

async fn run_server() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let config = DeliveryConfig::from_env();
    let state = AppState::new(config.clone())?;
    let shutdown = Arc::new(Notify::new());
    let ddns = DdnsConfig::from_env().map(|cfg| spawn_ddns_task(cfg, Arc::clone(&shutdown)));

    let router = app(state);
    let tls = config.tls_cert.as_ref().zip(config.tls_key.as_ref());
    tracing::info!(
        listen = %config.listen,
        tls = tls.is_some(),
        data_dir = %config.data_dir.display(),
        instance_id = %ghal_bol_delivery::instance_id(),
        min_ttl_secs = config.min_ttl_secs,
        max_ttl_secs = config.max_ttl_secs,
        quota_bytes = config.quota_bytes_per_peer,
        "ghal_bol_delivery listening"
    );

    let serve = if let Some((cert, key)) = tls {
        if !cert.is_file() {
            return Err(format!("TLS cert not found: {}", cert.display()).into());
        }
        if !key.is_file() {
            return Err(format!("TLS key not found: {}", key.display()).into());
        }
        let tls_config = RustlsConfig::from_pem_file(cert, key).await?;
        let handle = axum_server::bind_rustls(config.listen, tls_config)
            .serve(router.into_make_service());
        tokio::select! {
            r = handle => r?,
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("shutdown signal received");
            }
        }
    } else {
        let listener = TcpListener::bind(config.listen).await?;
        let handle = axum::serve(listener, router);
        tokio::select! {
            r = handle => r?,
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("shutdown signal received");
            }
        }
    };

    let _ = serve;

    shutdown.notify_waiters();
    if let Some(handle) = ddns {
        let _ = handle.await;
    }
    Ok(())
}
