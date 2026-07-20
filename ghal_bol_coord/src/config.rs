use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

/// Coord server data root — separate from the Flutter app (`com.ghalbol`).
/// Keep `com.ghalbol.coord` / `ghalbol_server` for coord1 home installs (main branch paths).
pub const DATA_NAMESPACE: &str = "com.ghalbol.coord";

/// Subdirectory under the app data root for coordination server files.
pub const SERVER_DATA_DIR: &str = "ghalbol_server";

const DB_FILE_NAME: &str = "coord.db";

/// `~/.local/share/com.ghal_bol.coord/ghal_bol_coord/coord.db` on Linux.
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
    /// Public HTTPS base for bridge connect URLs (e.g. `https://coord.ghalbol.com`).
    pub public_base_url: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            listen: "0.0.0.0:8765".parse().expect("valid default listen"),
            database_path: default_database_path(),
            challenge_ttl: Duration::from_secs(120),
            presence_ttl: Duration::from_secs(90),
            purge_interval: Duration::from_secs(30),
            public_base_url: "https://coord.ghalbol.com".to_string(),
        }
    }
}

impl ServerConfig {
    pub fn from_env() -> Self {
        let mut cfg = Self::default();
        if let Ok(s) = std::env::var("GHAL_BOL_COORD_LISTEN")
            .or_else(|_| std::env::var("GHAL_BOL_SERVER_LISTEN"))
        {
            if let Ok(addr) = s.parse() {
                cfg.listen = addr;
            } else {
                tracing::warn!(listen = %s, "GHAL_BOL_COORD_LISTEN invalid; using default");
            }
        }
        if let Ok(s) = std::env::var("GHAL_BOL_COORD_DB")
            .or_else(|_| std::env::var("GHAL_BOL_SERVER_DB"))
        {
            let t = s.trim();
            if !t.is_empty() {
                let p = PathBuf::from(t);
                cfg.database_path = if p.extension().is_some() || t.ends_with(".db") {
                    p
                } else {
                    // Directory override → place `coord.db` inside (e.g. …/ghal_bol_coord).
                    p.join(DB_FILE_NAME)
                };
            }
        }
        if let Ok(s) = std::env::var("GHAL_BOL_COORD_CHALLENGE_TTL_SECS")
            .or_else(|_| std::env::var("GHAL_BOL_SERVER_CHALLENGE_TTL_SECS"))
        {
            if let Ok(secs) = s.parse::<u64>() {
                cfg.challenge_ttl = Duration::from_secs(secs);
            }
        }
        if let Ok(s) = std::env::var("GHAL_BOL_COORD_PRESENCE_TTL_SECS")
            .or_else(|_| std::env::var("GHAL_BOL_SERVER_PRESENCE_TTL_SECS"))
        {
            if let Ok(secs) = s.parse::<u64>() {
                cfg.presence_ttl = Duration::from_secs(secs);
            }
        }
        if let Ok(s) = std::env::var("GHAL_BOL_COORD_PURGE_INTERVAL_SECS")
            .or_else(|_| std::env::var("GHAL_BOL_SERVER_PURGE_INTERVAL_SECS"))
        {
            if let Ok(secs) = s.parse::<u64>() {
                cfg.purge_interval = Duration::from_secs(secs.max(5));
            }
        }
        if let Ok(s) = std::env::var("GHAL_BOL_COORD_PUBLIC_URL")
            .or_else(|_| std::env::var("GHAL_BOL_COORD_BASE_URL"))
        {
            let t = s.trim().trim_end_matches('/');
            if !t.is_empty() {
                cfg.public_base_url = t.to_string();
            }
        }
        cfg
    }
}
