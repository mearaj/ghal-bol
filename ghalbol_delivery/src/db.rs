//! SQLite schema for delivery mailbox.

use crate::error::{DeliveryError, Result};
use rusqlite::Connection;
use std::path::Path;

pub const SCHEMA_VERSION: i32 = 2;

pub fn open_and_migrate(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
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

pub fn migrate(conn: &Connection) -> Result<()> {
    let version: i32 = conn
        .query_row("SELECT version FROM schema_version LIMIT 1", [], |r| r.get(0))
        .unwrap_or(0);

    if version == 0 {
        conn.execute_batch(
            "CREATE TABLE schema_version (version INTEGER NOT NULL);
             CREATE TABLE pending_messages (
                sender_wire TEXT NOT NULL,
                message_id TEXT NOT NULL,
                recipient_wire TEXT NOT NULL,
                envelope_blob TEXT,
                size_bytes INTEGER NOT NULL,
                uploaded_at_ms INTEGER NOT NULL,
                expires_at_ms INTEGER NOT NULL,
                state TEXT NOT NULL,
                delivered_at_ms INTEGER,
                expired_at_ms INTEGER,
                read_at_ms INTEGER,
                PRIMARY KEY (sender_wire, message_id)
             );
             CREATE INDEX idx_pending_recipient_state
                ON pending_messages(recipient_wire, state);
             CREATE INDEX idx_pending_expires
                ON pending_messages(state, expires_at_ms);
             CREATE INDEX idx_pending_sender_state
                ON pending_messages(sender_wire, state);
             INSERT INTO schema_version (version) VALUES (2);",
        )?;
        return Ok(());
    }

    if version == 1 {
        conn.execute_batch(
            "ALTER TABLE pending_messages ADD COLUMN read_at_ms INTEGER;
             UPDATE schema_version SET version = 2;",
        )?;
        return Ok(());
    }

    if version != SCHEMA_VERSION {
        return Err(DeliveryError::Internal(format!(
            "unsupported schema version {version} (expected {SCHEMA_VERSION})"
        )));
    }
    Ok(())
}

pub fn open_memory() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    conn.execute_batch(
        "PRAGMA foreign_keys=ON;
         PRAGMA journal_mode=MEMORY;",
    )?;
    migrate(&conn)?;
    Ok(conn)
}
