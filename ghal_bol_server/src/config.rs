use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

/// Same Linux data root as `ghal_bol` / `ghal_bol_ui` (`com.ghalbol`).
pub const DATA_NAMESPACE: &str = "com.ghalbol";

/// Subdirectory under the app data root for coordination server files.
pub const SERVER_DATA_DIR: &str = "ghalbol_server";

const DB_FILE_NAME: &str = "coord.db";

/// `~/.local/share/com.ghalbol/ghalbol_server/coord.db` on Linux.
pub fn default_database_path() -> PathBuf {
    let mut base = directories::ProjectDirs::from_path(PathBuf::from(DATA_NAMESPACE))
        .map(|d| d.data_local_dir().to_path_buf())
        .unwrap_or_else(fallback_data_root);
    base.push(SERVER_DATA_DIR);
    base.push(DB_FILE_NAME);
    base
}

fn fallback_data_root() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home)
        .join(".local/share")
        .join(DATA_NAMESPACE)
}

/// Runtime configuration for [`crate::AppState`].
#[derive(Clone, Debug)]
pub struct ServerConfig {
    pub listen: SocketAddr,
    pub database_path: PathBuf,
    /// How long a registration challenge stays valid.
    pub challenge_ttl: Duration,
    /// Peer records older than this (by last heartbeat) are offline / purged.
    pub presence_ttl: Duration,
    /// Background purge interval.
    pub purge_interval: Duration,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            listen: "0.0.0.0:8765".parse().expect("valid default listen"),
            database_path: default_database_path(),
            challenge_ttl: Duration::from_secs(120),
            presence_ttl: Duration::from_secs(90),
            purge_interval: Duration::from_secs(30),
        }
    }
}

impl ServerConfig {
    pub fn from_env() -> Self {
        let mut cfg = Self::default();
        if let Ok(s) = std::env::var("GHAL_BOL_SERVER_LISTEN") {
            if let Ok(addr) = s.parse() {
                cfg.listen = addr;
            } else {
                tracing::warn!(listen = %s, "GHAL_BOL_SERVER_LISTEN invalid; using default");
            }
        }
        if let Ok(s) = std::env::var("GHAL_BOL_SERVER_DB") {
            let t = s.trim();
            if !t.is_empty() {
                let p = PathBuf::from(t);
                cfg.database_path = if p.extension().is_some() || t.ends_with(".db") {
                    p
                } else {
                    // Directory override → place `coord.db` inside (e.g. …/ghalbol_server).
                    p.join(DB_FILE_NAME)
                };
            }
        }
        if let Ok(s) = std::env::var("GHAL_BOL_SERVER_CHALLENGE_TTL_SECS") {
            if let Ok(secs) = s.parse::<u64>() {
                cfg.challenge_ttl = Duration::from_secs(secs);
            }
        }
        if let Ok(s) = std::env::var("GHAL_BOL_SERVER_PRESENCE_TTL_SECS") {
            if let Ok(secs) = s.parse::<u64>() {
                cfg.presence_ttl = Duration::from_secs(secs);
            }
        }
        if let Ok(s) = std::env::var("GHAL_BOL_SERVER_PURGE_INTERVAL_SECS") {
            if let Ok(secs) = s.parse::<u64>() {
                cfg.purge_interval = Duration::from_secs(secs.max(5));
            }
        }
        cfg
    }
}
