//! Reads the Flutter `chat_transcript_v1.json` format so native P2P can restore outbound work
//! without the UI walking history on the main isolate.
//!
//! All file access goes through [`crate::dm_transcript_store`] locking — do not read/write this
//! path directly (concurrent append/patch in `:p2p` used to clobber rows).
//!
//! ## Outbound delivery vs read (see `docs/GHAL_BOL_DM_MSG_V1.md`)
//!
//! Native P2P rescans this file on a ~1s tick and resends rows still needing **`ack_received`**.
//! The **recipient** decides read (`ack_read`); the sender learns via ack frames, not shared DB.
//!
//! | Transcript `delivery` (outgoing) | Meaning on sender | P2P resend? |
//! |----------------------------------|-------------------|-------------|
//! | `pending`, `sent`, `failed`      | not yet delivered | yes         |
//! | `delivered`                      | peer got the text | no          |
//! | `read`                           | peer read the text| no          |

use std::path::Path;

use serde_json::Value;
use thiserror::Error;

use crate::dm_transcript_store::{
    patch_inbound_read_ack_sent_at_path, read_root_unlocked, with_transcript_path,
};

#[derive(Clone, Debug)]
pub struct PendingInboundReadAckRow {
    pub conversation_key: String,
    pub message_id: String,
}

#[derive(Clone, Debug)]
pub struct PendingOutboundRow {
    /// Thread key in the transcript file (libp2p peer id or signing pk hex).
    pub conversation_key: String,
    pub message_id: String,
    pub text: String,
    pub created_at_ms: i64,
}

#[derive(Debug, Error)]
pub enum TranscriptError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("json: {0}")]
    Json(#[from] serde_json::Error),

    #[error("transcript root is not a JSON object")]
    NotObject,
}

fn pending_outbound_from_root(
    root: &Value,
    app_namespace: &str,
) -> Result<Vec<PendingOutboundRow>, TranscriptError> {
    let Some(ns_obj) = root.get(app_namespace) else {
        return Ok(Vec::new());
    };
    let Some(threads) = ns_obj.as_object() else {
        return Err(TranscriptError::NotObject);
    };

    let mut out = Vec::new();
    for (conversation_key, thread) in threads {
        let Some(lines) = thread.as_array() else {
            continue;
        };
        for line in lines.iter().rev() {
            let Some(obj) = line.as_object() else {
                continue;
            };
            if obj.get("outgoing").and_then(|v| v.as_bool()) != Some(true) {
                continue;
            }
            let delivery = obj
                .get("delivery")
                .and_then(|v| v.as_str())
                .unwrap_or("pending");
            if delivery == "delivered" || delivery == "read" {
                continue;
            }
            let message_id = obj
                .get("message_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            if message_id.is_empty() {
                continue;
            }
            let text = obj
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if text.is_empty() {
                continue;
            }
            let created_at_ms = obj
                .get("created_at_ms")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            out.push(PendingOutboundRow {
                conversation_key: conversation_key.clone(),
                message_id,
                text,
                created_at_ms,
            });
        }
    }
    Ok(out)
}

fn pending_inbound_read_ack_from_root(
    root: &Value,
    app_namespace: &str,
) -> Result<Vec<PendingInboundReadAckRow>, TranscriptError> {
    let Some(ns_obj) = root.get(app_namespace) else {
        return Ok(Vec::new());
    };
    let Some(threads) = ns_obj.as_object() else {
        return Err(TranscriptError::NotObject);
    };
    let mut out = Vec::new();
    for (conversation_key, thread) in threads {
        let Some(lines) = thread.as_array() else {
            continue;
        };
        for line in lines {
            let Some(obj) = line.as_object() else {
                continue;
            };
            if obj.get("outgoing").and_then(|v| v.as_bool()) == Some(true) {
                continue;
            }
            if obj.get("read_ack_sent").and_then(|v| v.as_bool()) == Some(true) {
                continue;
            }
            let message_id = obj
                .get("message_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            if message_id.is_empty() {
                continue;
            }
            out.push(PendingInboundReadAckRow {
                conversation_key: conversation_key.clone(),
                message_id,
            });
        }
    }
    Ok(out)
}

/// Pending outbound rows for [app_namespace] (all conversations), newest first per thread.
pub fn pending_outbound_rows(
    path: &Path,
    app_namespace: &str,
) -> Result<Vec<PendingOutboundRow>, TranscriptError> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    with_transcript_path(path, |path| {
        let root = read_root_unlocked(path)?;
        pending_outbound_from_root(&root, app_namespace).map_err(|e| {
            crate::dm_transcript_store::TranscriptStoreError::Io(std::io::Error::other(
                e.to_string(),
            ))
        })
    })
    .map_err(|e| TranscriptError::Io(std::io::Error::other(e.to_string())))
}

/// Inbound rows still needing `ack_read` (`read_ack_sent` = peer confirmed our read receipt).
pub fn pending_inbound_read_ack_rows(
    path: &Path,
    app_namespace: &str,
) -> Result<Vec<PendingInboundReadAckRow>, TranscriptError> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    with_transcript_path(path, |path| {
        let root = read_root_unlocked(path)?;
        pending_inbound_read_ack_from_root(&root, app_namespace).map_err(|e| {
            crate::dm_transcript_store::TranscriptStoreError::Io(std::io::Error::other(
                e.to_string(),
            ))
        })
    })
    .map_err(|e| TranscriptError::Io(std::io::Error::other(e.to_string())))
}

/// Mark one inbound row as read-acked on disk (best-effort; keeps Flutter/native in sync after restart).
#[allow(dead_code)] // path-based API; prefer `dm_transcript_store::patch_inbound_read_ack_sent_at_path`
pub fn mark_inbound_read_ack_sent(
    path: &Path,
    app_namespace: &str,
    message_id: &str,
) -> Result<bool, TranscriptError> {
    patch_inbound_read_ack_sent_at_path(path, app_namespace, message_id)
        .map_err(|e| TranscriptError::Io(std::io::Error::other(e.to_string())))
}
