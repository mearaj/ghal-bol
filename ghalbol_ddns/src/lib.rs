//! GoDaddy dynamic DNS for Ghal Bol home servers (coord, delivery, …).
//!
//! Uses GoDaddy REST **v3** with a Personal Access Token (PAT).
//! Runs in-process so DNS stays in sync with the server — no separate systemd timer.

use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::sync::Notify;
use tracing::{info, warn};

const GODADDY_V3_BASE: &str = "https://api.godaddy.com/v3";

/// Parsed GoDaddy DDNS settings (credentials file + optional env overrides).
#[derive(Clone, Debug)]
pub struct DdnsConfig {
    pub pat: String,
    pub domain: String,
    pub host: String,
    pub ttl: u32,
    pub poll_interval: Duration,
    pub state_path: PathBuf,
}

impl DdnsConfig {
    /// Load from `GHALBOL_DDNS_CREDENTIALS` (path to key=value file). Disabled when unset or missing.
    pub fn from_env() -> Option<Self> {
        let creds_path = std::env::var("GHALBOL_DDNS_CREDENTIALS")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())?;
        Self::from_credentials_file(&creds_path)
    }

    /// Parse a GoDaddy credentials file (`GODADDY_*` key=value lines).
    pub fn from_credentials_file(path: &str) -> Option<Self> {
        let path = PathBuf::from(path);
        if !path.is_file() {
            warn!(
                path = %path.display(),
                "GHALBOL_DDNS_CREDENTIALS set but file missing — DDNS disabled"
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
        if get("GODADDY_API_KEY").is_some() || get("GODADDY_API_SECRET").is_some() {
            warn!(
                path = %path.display(),
                "GoDaddy DDNS: v1 API keys (GODADDY_API_KEY/SECRET) are no longer supported — use GODADDY_PAT (v3 Personal Access Token)"
            );
            return None;
        }
        let pat = get("GODADDY_PAT")?;
        let domain = get("GODADDY_DOMAIN")?;
        let host = get("GODADDY_HOST")?;
        let ttl = vars
            .get("GODADDY_TTL")
            .and_then(|s| s.parse().ok())
            .unwrap_or(600)
            .max(600);
        let poll_secs = std::env::var("GHALBOL_DDNS_POLL_SECS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(300)
            .max(60);
        let state_path = std::env::var("GHALBOL_DDNS_STATE_FILE")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| default_state_path(&path, &host));
        Some(Self {
            pat,
            domain,
            host,
            ttl,
            poll_interval: Duration::from_secs(poll_secs),
            state_path,
        })
    }

    pub fn fqdn(&self) -> String {
        if self.host == "@" {
            self.domain.clone()
        } else {
            format!("{}.{}", self.host, self.domain)
        }
    }

    fn list_a_records_url(&self) -> String {
        format!(
            "{GODADDY_V3_BASE}/domains/zones/{}/dns-records?type=A&name={}",
            self.domain, self.host
        )
    }

    fn create_record_url(&self) -> String {
        format!(
            "{GODADDY_V3_BASE}/domains/zones/{}/dns-records",
            self.domain
        )
    }

    fn replace_record_url(&self, record_id: &str) -> String {
        format!(
            "{GODADDY_V3_BASE}/domains/zones/{}/dns-records/{record_id}",
            self.domain
        )
    }
}

fn default_state_path(creds_path: &Path, host: &str) -> PathBuf {
    creds_path
        .parent()
        .map(|p| p.join(format!(".godaddy-ddns-{host}.last_ip")))
        .unwrap_or_else(|| PathBuf::from(format!(".godaddy-ddns-{host}.last_ip")))
}

/// One-shot DDNS update (manual / emergency). Returns Ok when A record matches public IPv4.
pub async fn run_ddns_pass(config: &DdnsConfig) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;
    run_pass(&client, config).await
}

/// Background task: poll public IPv4 vs GoDaddy A record; update when stale.
pub fn spawn_ddns_task(
    config: DdnsConfig,
    shutdown: std::sync::Arc<Notify>,
) -> tokio::task::JoinHandle<()> {
    let fqdn = config.fqdn();
    info!(
        fqdn = %fqdn,
        poll_secs = config.poll_interval.as_secs(),
        "GoDaddy DDNS enabled (in-process, API v3)"
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
    let remote = fetch_godaddy_a_record(client, config)
        .await
        .unwrap_or_default();
    if remote == current {
        write_state(&config.state_path, &current)?;
        return Ok(());
    }
    upsert_godaddy_a_record(client, config, &current).await?;
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

#[derive(Debug)]
struct GodaddyARecord {
    record_id: String,
    data: String,
}

async fn fetch_godaddy_a_record(
    client: &reqwest::Client,
    config: &DdnsConfig,
) -> Result<String, String> {
    match list_godaddy_a_records(client, config).await? {
        Some(row) => Ok(row.data),
        None => Ok(String::new()),
    }
}

async fn list_godaddy_a_records(
    client: &reqwest::Client,
    config: &DdnsConfig,
) -> Result<Option<GodaddyARecord>, String> {
    let url = config.list_a_records_url();
    let resp = client
        .get(&url)
        .header("Authorization", auth_header(config))
        .header("Cache-Control", "no-cache")
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("GoDaddy GET {url} {}", resp.status()));
    }
    let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    let items = body
        .get("items")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let first = items.first();
    Ok(first.and_then(|v| {
        let record_id = v.get("recordId")?.as_str()?.to_string();
        let data = v.get("data")?.as_str()?.to_string();
        Some(GodaddyARecord { record_id, data })
    }))
}

async fn upsert_godaddy_a_record(
    client: &reqwest::Client,
    config: &DdnsConfig,
    ip: &str,
) -> Result<(), String> {
    let body = serde_json::json!({
        "name": config.host,
        "type": "A",
        "data": ip,
        "ttl": config.ttl,
    });
    if let Some(existing) = list_godaddy_a_records(client, config).await? {
        let url = config.replace_record_url(&existing.record_id);
        let resp = client
            .put(&url)
            .header("Authorization", auth_header(config))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if resp.status().is_success() {
            return Ok(());
        }
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("GoDaddy PUT {url} failed {status}: {text}"));
    }
    let url = config.create_record_url();
    let resp = client
        .post(&url)
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
        Err(format!("GoDaddy POST {url} failed {status}: {text}"))
    }
}

fn auth_header(config: &DdnsConfig) -> String {
    format!("Bearer {}", config.pat)
}

fn write_state(path: &Path, ip: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
    }
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
            "GODADDY_PAT=pat789\nGODADDY_DOMAIN=ghalbol.com\nGODADDY_HOST=delivery\n"
        )
        .unwrap();
        let cfg = DdnsConfig::from_credentials_file(path.to_str().unwrap()).expect("parse");
        assert_eq!(cfg.pat, "pat789");
        assert_eq!(cfg.domain, "ghalbol.com");
        assert_eq!(cfg.host, "delivery");
        assert_eq!(cfg.fqdn(), "delivery.ghalbol.com");
        assert!(cfg
            .state_path
            .to_string_lossy()
            .contains(".godaddy-ddns-delivery.last_ip"));
    }

    #[test]
    fn rejects_legacy_api_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("creds");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(
            f,
            "GODADDY_API_KEY=key123\nGODADDY_API_SECRET=sec456\nGODADDY_DOMAIN=ghalbol.com\nGODADDY_HOST=coord1\n"
        )
        .unwrap();
        assert!(DdnsConfig::from_credentials_file(path.to_str().unwrap()).is_none());
    }
}
