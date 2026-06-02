//! Ghal Bol coordination server — presence and endpoint discovery only.
//!
//! Does not store message content or transcripts. See workspace `README.md`.

mod auth;
mod config;
mod db;
mod error;
mod presence;
mod routes;

pub use auth::registration_message_digest;
pub use config::ServerConfig;
pub use error::ServerError;
pub use presence::{PeerEndpoint, PeerRecord, PresenceStore};

use axum::Router;
use std::sync::Arc;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::trace::TraceLayer;

/// Max JSON body size for API requests (64 KiB).
pub const MAX_REQUEST_BODY_BYTES: usize = 64 * 1024;

/// Shared application state.
pub struct AppState {
    pub config: ServerConfig,
    pub presence: Arc<PresenceStore>,
}

impl AppState {
    pub fn open(config: ServerConfig) -> Result<Self, ServerError> {
        let presence = PresenceStore::open(&config.database_path)?;
        tracing::info!(db = %config.database_path.display(), "sqlite ready");
        Ok(Self {
            config,
            presence: Arc::new(presence),
        })
    }

    /// Used by integration tests (`:memory:` SQLite).
    pub fn open_in_memory(config: ServerConfig) -> Result<Self, ServerError> {
        let presence = PresenceStore::open_in_memory()?;
        Ok(Self {
            config,
            presence: Arc::new(presence),
        })
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
