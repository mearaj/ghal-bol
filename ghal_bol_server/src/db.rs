//! SQLite schema for peer presence (coordination only).

use crate::error::ServerError;
use rusqlite::Connection;
use std::path::Path;

pub const SCHEMA_VERSION: i32 = 1;

pub fn open_and_migrate(path: &Path) -> Result<Connection, ServerError> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| ServerError::Internal(format!("create db directory: {e}")))?;
        }
    }
    let conn = Connection::open(path)?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA synchronous=NORMAL;
         PRAGMA foreign_keys=ON;
         PRAGMA busy_timeout=5000;",
    )?;
    migrate(&conn)?;
    Ok(conn)
}

pub fn migrate(conn: &Connection) -> Result<(), ServerError> {
    let version: i32 = conn
        .query_row("SELECT version FROM schema_version LIMIT 1", [], |r| {
            r.get(0)
        })
        .unwrap_or(0);

    if version == 0 {
        conn.execute_batch(
            "CREATE TABLE schema_version (
                version INTEGER NOT NULL
            );
            CREATE TABLE peers (
                public_key_hex TEXT PRIMARY KEY NOT NULL,
                endpoints_json TEXT NOT NULL,
                transport_capabilities_json TEXT NOT NULL,
                ipv6 TEXT,
                ipv4 TEXT,
                last_heartbeat_unix_ms INTEGER NOT NULL
            );
            CREATE INDEX idx_peers_last_heartbeat
                ON peers(last_heartbeat_unix_ms);
            INSERT INTO schema_version (version) VALUES (1);",
        )?;
        return Ok(());
    }

    if version != SCHEMA_VERSION {
        return Err(ServerError::Internal(format!(
            "unsupported schema version {version} (expected {SCHEMA_VERSION})"
        )));
    }
    Ok(())
}

pub fn open_memory() -> Result<Connection, ServerError> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch(
        "PRAGMA foreign_keys=ON;
         CREATE TABLE schema_version (version INTEGER NOT NULL);
         CREATE TABLE peers (
            public_key_hex TEXT PRIMARY KEY NOT NULL,
            endpoints_json TEXT NOT NULL,
            transport_capabilities_json TEXT NOT NULL,
            ipv6 TEXT,
            ipv4 TEXT,
            last_heartbeat_unix_ms INTEGER NOT NULL
         );
         CREATE INDEX idx_peers_last_heartbeat ON peers(last_heartbeat_unix_ms);
         INSERT INTO schema_version (version) VALUES (1);",
    )?;
    Ok(conn)
}
