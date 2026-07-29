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

use thiserror::Error;

#[derive(Debug, Error)]
pub enum TranscriptError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}
