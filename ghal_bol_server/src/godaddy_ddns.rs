//! GoDaddy dynamic DNS for home coord (`coord1.ghalbol.com`).
//!
//! Runs inside `ghal_bol_server` so DNS stays in sync with the same process that
//! serves coord/relay — no separate systemd timer that can stall independently.

use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::sync::Notify;
use tracing::{info, warn};

/// Parsed GoDaddy DDNS settings (from credentials file + optional env overrides).
#[derive(Clone, Debug)]
pub struct DdnsConfig {
    pub api_key: String,
    pub api_secret: String,
    pub domain: String,
    pub host: String,
    pub ttl: u32,
    pub poll_interval: Duration,
    pub state_path: PathBuf,
}

impl DdnsConfig {
    /// Load from `GHAL_BOL_DDNS_CREDENTIALS` (path to key=value file). Disabled when unset or missing.
    pub fn from_env() -> Option<Self> {
        let creds_path = std::env::var("GHAL_BOL_DDNS_CREDENTIALS")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())?;
        Self::from_credentials_file(&creds_path)
    }

    fn from_credentials_file(path: &str) -> Option<Self> {
        let path = PathBuf::from(path);
        if !path.is_file() {
            warn!(
                path = %path.display(),
                "GHAL_BOL_DDNS_CREDENTIALS set but file missing — DDNS disabled"
            );
            return None;
        }
        let raw = std::fs::read_to_string(&path).ok()?;
        let mut vars = std::collections::HashMap::<String, String>::new();
        for line in raw.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            vars.insert(k.trim().to_string(), v.trim().to_string());
        }
        let get = |key: &str| vars.get(key).cloned().filter(|v| !v.is_empty() && !v.contains("paste_"));
        let api_key = get("GODADDY_API_KEY")?;
        let api_secret = get("GODADDY_API_SECRET")?;
        let domain = get("GODADDY_DOMAIN")?;
        let host = get("GODADDY_HOST")?;
        let ttl = vars
            .get("GODADDY_TTL")
            .and_then(|s| s.parse().ok())
            .unwrap_or(600);
        let poll_secs = std::env::var("GHAL_BOL_DDNS_POLL_SECS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(300)
            .max(60);
        let state_path = path
            .parent()
            .map(|p| p.join(".godaddy-ddns-coord1.last_ip"))
            .unwrap_or_else(|| PathBuf::from(".godaddy-ddns-coord1.last_ip"));
        Some(Self {
            api_key,
            api_secret,
            domain,
            host,
            ttl,
            poll_interval: Duration::from_secs(poll_secs),
            state_path,
        })
    }

    fn fqdn(&self) -> String {
        format!("{}.{}", self.host, self.domain)
    }

    fn records_url(&self) -> String {
        format!(
            "https://api.godaddy.com/v1/domains/{}/records/A/{}",
            self.domain, self.host
        )
    }
}

/// Background task: poll public IPv4 vs GoDaddy A record; PUT when stale.
pub fn spawn_ddns_task(config: DdnsConfig, shutdown: std::sync::Arc<Notify>) -> tokio::task::JoinHandle<()> {
    let fqdn = config.fqdn();
    info!(
        fqdn = %fqdn,
        poll_secs = config.poll_interval.as_secs(),
        "GoDaddy DDNS enabled (in-process)"
    );
    tokio::spawn(async move {
        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                warn!(error = %e, "GoDaddy DDNS: failed to build HTTP client");
                return;
            }
        };
        let mut tick = tokio::time::interval(config.poll_interval);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // First tick fires immediately — sync DNS on server start.
        loop {
            tokio::select! {
                _ = shutdown.notified() => {
                    tracing::debug!("GoDaddy DDNS task stopped");
                    break;
                }
                _ = tick.tick() => {
                    if let Err(e) = run_pass(&client, &config).await {
                        warn!(error = %e, fqdn = %fqdn, "GoDaddy DDNS pass failed");
                    }
                }
            }
        }
    })
}

async fn run_pass(client: &reqwest::Client, config: &DdnsConfig) -> Result<(), String> {
    let current = fetch_public_ipv4(client).await?;
    let remote = fetch_godaddy_a_record(client, config).await.unwrap_or_default();
    if remote == current {
        write_state(&config.state_path, &current)?;
        return Ok(());
    }
    put_godaddy_a_record(client, config, &current).await?;
    write_state(&config.state_path, &current)?;
    info!(
        fqdn = %config.fqdn(),
        old = %if remote.is_empty() { "(none)" } else { &remote },
        new = %current,
        "GoDaddy DDNS updated A record"
    );
    Ok(())
}

async fn fetch_public_ipv4(client: &reqwest::Client) -> Result<String, String> {
    for url in ["https://api.ipify.org", "https://ifconfig.me/ip"] {
        match client.get(url).send().await {
            Ok(resp) if resp.status().is_success() => {
                let body = resp.text().await.map_err(|e| e.to_string())?;
                let ip = body.trim().to_string();
                if !ip.is_empty() {
                    return Ok(ip);
                }
            }
            Ok(resp) => {
                warn!(url, status = %resp.status(), "public IP probe failed");
            }
            Err(e) => {
                warn!(url, error = %e, "public IP probe error");
            }
        }
    }
    Err("could not detect public IPv4".into())
}

async fn fetch_godaddy_a_record(client: &reqwest::Client, config: &DdnsConfig) -> Result<String, String> {
    let resp = client
        .get(&config.records_url())
        .header("Authorization", auth_header(config))
        .header("Cache-Control", "no-cache")
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("GoDaddy GET {} {}", config.records_url(), resp.status()));
    }
    let rows: Vec<serde_json::Value> = resp.json().await.map_err(|e| e.to_string())?;
    Ok(rows
        .first()
        .and_then(|v| v.get("data"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string())
}

async fn put_godaddy_a_record(
    client: &reqwest::Client,
    config: &DdnsConfig,
    ip: &str,
) -> Result<(), String> {
    let body = serde_json::json!([{"data": ip, "ttl": config.ttl}]);
    let resp = client
        .put(&config.records_url())
        .header("Authorization", auth_header(config))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if resp.status().is_success() {
        Ok(())
    } else {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        Err(format!("GoDaddy PUT failed {status}: {text}"))
    }
}

fn auth_header(config: &DdnsConfig) -> String {
    format!("sso-key {}:{}", config.api_key, config.api_secret)
}

fn write_state(path: &Path, ip: &str) -> Result<(), String> {
    std::fs::write(path, format!("{ip}\n")).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn parse_credentials_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("creds");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(
            f,
            "GODADDY_API_KEY=key123\nGODADDY_API_SECRET=sec456\nGODADDY_DOMAIN=ghalbol.com\nGODADDY_HOST=coord1\n"
        )
        .unwrap();
        let cfg = DdnsConfig::from_credentials_file(path.to_str().unwrap()).expect("parse");
        assert_eq!(cfg.api_key, "key123");
        assert_eq!(cfg.domain, "ghalbol.com");
        assert_eq!(cfg.host, "coord1");
        assert_eq!(cfg.fqdn(), "coord1.ghalbol.com");
    }
}
