//! Delivery envelope validation (opaque E2E blob).

use crate::auth::verify_signature;
use crate::error::{DeliveryError, Result};
use crate::identity::normalize_identity_wire;
use serde_json::Value;

pub const DELIVERY_MSG_SHARE: &str = "ghal_bol_delivery_msg_v1";
pub const DELIVERY_MSG_FORMAT_VERSION: u64 = 1;

pub fn envelope_sign_bytes(
    message_id: &str,
    sender_wire: &str,
    recipient_wire: &str,
    created_at_ms: i64,
    ciphertext_hex: &str,
) -> Vec<u8> {
    let body = serde_json::json!({
        "ciphertext_hex": ciphertext_hex,
        "created_at_ms": created_at_ms,
        "message_id": message_id,
        "recipient_wire": recipient_wire.trim().to_ascii_lowercase(),
        "sender_wire": sender_wire.trim().to_ascii_lowercase(),
    });
    serde_json::to_vec(&body).unwrap_or_default()
}

pub fn validate_envelope(
    envelope: &Value,
    session_sender_wire: &str,
) -> Result<ValidatedEnvelope> {
    let share = envelope
        .get("ghalbol.share")
        .and_then(|v| v.as_str())
        .ok_or_else(|| DeliveryError::InvalidEnvelope("missing ghalbol.share".into()))?;
    if share != DELIVERY_MSG_SHARE {
        return Err(DeliveryError::InvalidEnvelope(format!(
            "unknown ghalbol.share: {share}"
        )));
    }
    let fv = envelope
        .get("format_version")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| DeliveryError::InvalidEnvelope("missing format_version".into()))?;
    if fv != DELIVERY_MSG_FORMAT_VERSION {
        return Err(DeliveryError::InvalidEnvelope(format!(
            "unsupported format_version {fv}"
        )));
    }
    let message_id = envelope
        .get("message_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| DeliveryError::InvalidEnvelope("missing message_id".into()))?
        .to_string();
    let sender_wire = envelope
        .get("sender_wire")
        .and_then(|v| v.as_str())
        .ok_or_else(|| DeliveryError::InvalidEnvelope("missing sender_wire".into()))?;
    let sender_norm = normalize_identity_wire(sender_wire)?;
    let session_norm = normalize_identity_wire(session_sender_wire)?;
    if sender_norm != session_norm {
        return Err(DeliveryError::Forbidden(
            "envelope sender does not match session".into(),
        ));
    }
    let recipient_wire = envelope
        .get("recipient_wire")
        .and_then(|v| v.as_str())
        .ok_or_else(|| DeliveryError::InvalidEnvelope("missing recipient_wire".into()))?;
    let recipient_norm = normalize_identity_wire(recipient_wire)?;
    let created_at_ms = envelope
        .get("created_at_ms")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| DeliveryError::InvalidEnvelope("missing created_at_ms".into()))?;
    let ciphertext_hex = envelope
        .get("ciphertext_hex")
        .and_then(|v| v.as_str())
        .ok_or_else(|| DeliveryError::InvalidEnvelope("missing ciphertext_hex".into()))?;
    if ciphertext_hex.trim().is_empty() {
        return Err(DeliveryError::InvalidEnvelope("empty ciphertext".into()));
    }
    let signature_hex = envelope
        .get("signature_hex")
        .and_then(|v| v.as_str())
        .ok_or_else(|| DeliveryError::InvalidEnvelope("missing signature_hex".into()))?;
    let sig = hex::decode(signature_hex.trim())
        .map_err(|e| DeliveryError::InvalidEnvelope(format!("signature hex: {e}")))?;
    let sign_bytes = envelope_sign_bytes(
        &message_id,
        &sender_norm,
        &recipient_norm,
        created_at_ms,
        ciphertext_hex,
    );
    verify_signature(&sender_norm, &sign_bytes, &sig)
        .map_err(|e| DeliveryError::InvalidEnvelope(e.to_string()))?;
    let blob = serde_json::to_string(envelope)
        .map_err(|e| DeliveryError::InvalidEnvelope(format!("envelope json: {e}")))?;
    let size_bytes = blob.len() as i64;
    Ok(ValidatedEnvelope {
        message_id,
        sender_wire: sender_norm,
        recipient_wire: recipient_norm,
        envelope_blob: blob,
        size_bytes,
    })
}

#[derive(Clone, Debug)]
pub struct ValidatedEnvelope {
    pub message_id: String,
    pub sender_wire: String,
    pub recipient_wire: String,
    pub envelope_blob: String,
    pub size_bytes: i64,
}
