use std::net::SocketAddr;
use std::path::PathBuf;

/// Delivery server data root — separate from the mobile app (`com.ghalbol`).
pub const DATA_NAMESPACE: &str = "com.ghal_bol.delivery";

/// Subdirectory under the app data root for delivery server files.
pub const DELIVERY_DATA_DIR: &str = "ghal_bol_delivery";

/// Runtime configuration for the delivery server.
#[derive(Clone, Debug)]
pub struct DeliveryConfig {
    pub listen: SocketAddr,
    pub data_dir: PathBuf,
    pub tls_cert: Option<PathBuf>,
    pub tls_key: Option<PathBuf>,
    pub min_ttl_secs: u64,
    pub max_ttl_secs: u64,
    pub default_ttl_secs: u64,
    pub quota_bytes_per_peer: u64,
}

impl Default for DeliveryConfig {
    fn default() -> Self {
        Self {
            listen: "0.0.0.0:8770".parse().expect("valid default listen"),
            data_dir: default_data_dir(),
            tls_cert: None,
            tls_key: None,
            min_ttl_secs: 3600,
            max_ttl_secs: 30 * 24 * 3600,
            default_ttl_secs: 7 * 24 * 3600,
            quota_bytes_per_peer: 500 * 1024 * 1024,
        }
    }
}

/// `~/.local/share/com.ghal_bol.delivery/ghal_bol_delivery/` on Linux.
pub fn default_data_dir() -> PathBuf {
    let mut base = directories::ProjectDirs::from_path(PathBuf::from(DATA_NAMESPACE))
        .map(|d| d.data_local_dir().to_path_buf())
        .unwrap_or_else(fallback_data_root);
    base.push(DELIVERY_DATA_DIR);
    base
}

fn fallback_data_root() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home)
        .join(".local/share")
        .join(DATA_NAMESPACE)
        .join(DELIVERY_DATA_DIR)
}

impl DeliveryConfig {
    pub fn from_env() -> Self {
        let mut cfg = Self::default();
        if let Ok(s) = std::env::var("GHAL_BOL_DELIVERY_LISTEN") {
            if let Ok(addr) = s.parse() {
                cfg.listen = addr;
            } else {
                tracing::warn!(listen = %s, "GHAL_BOL_DELIVERY_LISTEN invalid; using default");
            }
        }
        if let Ok(s) = std::env::var("GHAL_BOL_DELIVERY_DATA_DIR") {
            let t = s.trim();
            if !t.is_empty() {
                cfg.data_dir = PathBuf::from(t);
            }
        }
        if let Ok(s) = std::env::var("GHAL_BOL_DELIVERY_TLS_CERT") {
            let t = s.trim();
            if !t.is_empty() {
                cfg.tls_cert = Some(PathBuf::from(t));
            }
        }
        if let Ok(s) = std::env::var("GHAL_BOL_DELIVERY_TLS_KEY") {
            let t = s.trim();
            if !t.is_empty() {
                cfg.tls_key = Some(PathBuf::from(t));
            }
        }
        if let Ok(s) = std::env::var("GHAL_BOL_DELIVERY_MIN_TTL_SECS") {
            if let Ok(v) = s.parse() {
                cfg.min_ttl_secs = v;
            }
        }
        if let Ok(s) = std::env::var("GHAL_BOL_DELIVERY_MAX_TTL_SECS") {
            if let Ok(v) = s.parse() {
                cfg.max_ttl_secs = v;
            }
        }
        if let Ok(s) = std::env::var("GHAL_BOL_DELIVERY_DEFAULT_TTL_SECS") {
            if let Ok(v) = s.parse() {
                cfg.default_ttl_secs = v;
            }
        }
        if let Ok(s) = std::env::var("GHAL_BOL_DELIVERY_QUOTA_BYTES_PER_PEER") {
            if let Ok(v) = s.parse() {
                cfg.quota_bytes_per_peer = v;
            }
        }
        cfg
    }
}
