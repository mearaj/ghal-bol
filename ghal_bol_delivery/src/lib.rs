//! Ghal Bol delivery server — temporary encrypted message mailbox.
//!
//! See `docs/GHAL_BOL_DELIVERY.md` and `docs/GHAL_BOL_DELIVERY_WIRE_V1.md`.

mod auth;
mod config;
mod db;
mod envelope;
mod error;
mod identity;
mod instance;
mod mailbox_ops;
mod policy;
mod routes;
mod store;
mod ws;

pub use config::{DeliveryConfig, DELIVERY_DATA_DIR, DATA_NAMESPACE, default_data_dir};
pub use error::{DeliveryError, Result};
pub use envelope::{ValidatedEnvelope, validate_envelope};
pub use instance::instance_id;
pub use mailbox_ops::{export_mailbox, import_mailbox, mailbox_stats, MailboxStats};
pub use policy::PolicyLimits;
pub use routes::{router, spawn_background_tasks};
pub use store::MailboxStore;

use axum::Router;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use auth::ChallengeStore;
use ws::SessionRegistry;

/// Shared application state.
pub struct AppState {
    pub config: DeliveryConfig,
    pub store: Arc<MailboxStore>,
    pub policy: PolicyLimits,
    pub registry: SessionRegistry,
    pub challenges: Arc<Mutex<ChallengeStore>>,
}

impl AppState {
    pub fn new(config: DeliveryConfig) -> Result<Arc<Self>> {
        std::fs::create_dir_all(&config.data_dir)?;
        let db_path = config.data_dir.join("mailbox.db");
        let conn = db::open_and_migrate(&db_path)?;
        let store = MailboxStore::shared(conn, &config);
        let policy = PolicyLimits::from_config(&config);
        Ok(Arc::new(Self {
            config,
            store,
            policy,
            registry: SessionRegistry::default(),
            challenges: Arc::new(Mutex::new(ChallengeStore::default())),
        }))
    }

    pub fn new_in_memory(config: DeliveryConfig) -> Result<Arc<Self>> {
        let conn = db::open_memory()?;
        let store = MailboxStore::shared(conn, &config);
        let policy = PolicyLimits::from_config(&config);
        Ok(Arc::new(Self {
            config,
            store,
            policy,
            registry: SessionRegistry::default(),
            challenges: Arc::new(Mutex::new(ChallengeStore::default())),
        }))
    }

    pub fn data_dir(&self) -> &PathBuf {
        &self.config.data_dir
    }
}

pub fn app(state: Arc<AppState>) -> Router {
    spawn_background_tasks(state.clone());
    router(state)
}
