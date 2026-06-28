//! Ghal Bol coordination server — presence and endpoint discovery only.
//!
//! Does not store message content or transcripts. See workspace `README.md`.

mod agent_pk;
mod auth;
mod config;
mod db;
mod endpoint_expand;
mod error;
mod godaddy_ddns;
mod presence;
mod relay_live;
mod relay_nat;
pub mod relay;
mod routes;

pub use auth::registration_message_digest;
pub use config::ServerConfig;
pub use error::ServerError;
pub use presence::{PeerEndpoint, PeerRecord, PresenceStore};
pub use godaddy_ddns::{DdnsConfig, spawn_ddns_task};
pub use relay_live::RelayLiveRegistry;
pub use relay::{RelayConfig, RelayInfo};

use axum::Router;
use std::sync::{Arc, Mutex};
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::trace::TraceLayer;

/// Max JSON body size for API requests (64 KiB).
pub const MAX_REQUEST_BODY_BYTES: usize = 64 * 1024;

/// Shared application state.
pub struct AppState {
    pub config: ServerConfig,
    pub presence: Arc<PresenceStore>,
    /// Relay coordinates advertised at `GET /v1/relay` (set once the relay node starts).
    pub relay_info: Mutex<Option<RelayInfo>>,
    /// Home UPnP dynamic relay — remap on client `/v1/relay` refetch (event-driven, not periodic poll).
    upnp_remap_tx: Mutex<Option<tokio::sync::mpsc::Sender<()>>>,
}

impl AppState {
    pub fn open(config: ServerConfig) -> Result<Self, ServerError> {
        let presence = PresenceStore::open(&config.database_path)?;
        tracing::info!(db = %config.database_path.display(), "sqlite ready");
        Ok(Self {
            config,
            presence: Arc::new(presence),
            relay_info: Mutex::new(None),
            upnp_remap_tx: Mutex::new(None),
        })
    }

    /// Used by integration tests (`:memory:` SQLite).
    pub fn open_in_memory(config: ServerConfig) -> Result<Self, ServerError> {
        let presence = PresenceStore::open_in_memory()?;
        Ok(Self {
            config,
            presence: Arc::new(presence),
            relay_info: Mutex::new(None),
            upnp_remap_tx: Mutex::new(None),
        })
    }

    /// Publish the relay coordinates returned by [`relay::start`].
    pub fn set_relay_info(&self, info: RelayInfo) {
        self.presence.set_relay_bootstrap_addrs(&info.addrs);
        if let Ok(mut g) = self.relay_info.lock() {
            *g = Some(info);
        }
    }

    pub(crate) fn set_upnp_remap_tx(&self, tx: tokio::sync::mpsc::Sender<()>) {
        if let Ok(mut g) = self.upnp_remap_tx.lock() {
            *g = Some(tx);
        }
    }

    /// Clients refetch `/v1/relay?remap=1` after bootstrap TCP failure — triggers throttled UPnP renew.
    pub fn request_upnp_remap(&self) {
        if let Ok(g) = self.upnp_remap_tx.lock() {
            if let Some(tx) = g.as_ref() {
                let _ = tx.try_send(());
            }
        }
    }
}

/// HTTP router (handlers only).
pub fn router(state: Arc<AppState>) -> Router {
    routes::router(state)
}

/// Production app: routes + body limit + request tracing.
pub fn app(state: Arc<AppState>) -> Router {
    router(state)
        .layer(RequestBodyLimitLayer::new(MAX_REQUEST_BODY_BYTES))
        .layer(TraceLayer::new_for_http())
}
