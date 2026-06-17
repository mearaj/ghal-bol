//! Ghal Bol coordination server — presence and endpoint discovery only.
//!
//! Does not store message content or transcripts. See workspace `README.md`.

mod agent_pk;
mod auth;
mod config;
mod db;
mod error;
mod presence;
pub mod relay;
mod routes;

pub use auth::registration_message_digest;
pub use config::ServerConfig;
pub use error::ServerError;
pub use presence::{PeerEndpoint, PeerRecord, PresenceStore};
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
}

impl AppState {
    pub fn open(config: ServerConfig) -> Result<Self, ServerError> {
        let presence = PresenceStore::open(&config.database_path)?;
        tracing::info!(db = %config.database_path.display(), "sqlite ready");
        Ok(Self {
            config,
            presence: Arc::new(presence),
            relay_info: Mutex::new(None),
        })
    }

    /// Used by integration tests (`:memory:` SQLite).
    pub fn open_in_memory(config: ServerConfig) -> Result<Self, ServerError> {
        let presence = PresenceStore::open_in_memory()?;
        Ok(Self {
            config,
            presence: Arc::new(presence),
            relay_info: Mutex::new(None),
        })
    }

    /// Publish the relay coordinates returned by [`relay::start`].
    pub fn set_relay_info(&self, info: RelayInfo) {
        if let Ok(mut g) = self.relay_info.lock() {
            *g = Some(info);
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
