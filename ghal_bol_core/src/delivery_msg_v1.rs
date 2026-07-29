//! Signed delivery envelope + identity-sealed inner text.

use serde_json::{Value, json};

use crate::delivery_auth::{sign_delivery_challenge, verify_delivery_signature};
use crate::identity::Identity;
use crate::keystore_v1::DecryptedIdentity;
use crate::offline_seal_v1::seal_to_secp256k1_public;
use crate::public_key_util::normalize_contact_identity_wire;

pub const DELIVERY_MSG_SHARE: &str = "ghal_bol_delivery_msg_v1";
pub const DELIVERY_MSG_FORMAT_VERSION: u64 = 1;

pub fn envelope_sign_bytes(
    message_id: &str,
    sender_wire: &str,
    recipient_wire: &str,
    created_at_ms: i64,
    ciphertext_hex: &str,
) -> Vec<u8> {
    let body = json!({
        "ciphertext_hex": ciphertext_hex,
        "created_at_ms": created_at_ms,
        "message_id": message_id,
        "recipient_wire": recipient_wire.trim().to_ascii_lowercase(),
        "sender_wire": sender_wire.trim().to_ascii_lowercase(),
    });
    serde_json::to_vec(&body).unwrap_or_default()
}

pub fn build_text_envelope(
    ident: &DecryptedIdentity,
    message_id: &str,
    recipient_wire: &str,
    text: &str,
    created_at_ms: i64,
) -> Result<Value, String> {
    let sender_wire = ident.identity_wire();
    let recipient_norm = normalize_contact_identity_wire(recipient_wire)?;
    let recipient_id = Identity::parse(&recipient_norm)?;
    if recipient_id.algorithm != crate::identity::IdentityAlgorithm::Secp256k1 {
        return Err("delivery cipher: secp256k1 recipients only in v1".to_string());
    }
    let inner = json!({ "text": text }).to_string();
    let sealed = seal_to_secp256k1_public(&recipient_id.public_key, inner.as_bytes())?;
    let ciphertext_hex = hex::encode(&sealed);
    let sign_bytes = envelope_sign_bytes(
        message_id,
        &sender_wire,
        &recipient_norm,
        created_at_ms,
        &ciphertext_hex,
    );
    let signature = sign_delivery_challenge(ident, &sign_bytes)?;
    Ok(json!({
        "ghalbol.share": DELIVERY_MSG_SHARE,
        "format_version": DELIVERY_MSG_FORMAT_VERSION,
        "message_id": message_id,
        "sender_wire": sender_wire,
        "recipient_wire": recipient_norm,
        "created_at_ms": created_at_ms,
        "ciphertext_hex": ciphertext_hex,
        "signature_hex": hex::encode(signature),
    }))
}

pub fn build_voice_envelope(
    ident: &DecryptedIdentity,
    message_id: &str,
    recipient_wire: &str,
    duration_ms: u32,
    opus_blob: &[u8],
    created_at_ms: i64,
) -> Result<Value, String> {
    let sender_wire = ident.identity_wire();
    let recipient_norm = normalize_contact_identity_wire(recipient_wire)?;
    let recipient_id = Identity::parse(&recipient_norm)?;
    if recipient_id.algorithm != crate::identity::IdentityAlgorithm::Secp256k1 {
        return Err("delivery cipher: secp256k1 recipients only in v1".to_string());
    }
    let inner = crate::voice_msg_v1::build_voice_inner(duration_ms, opus_blob)?;
    let inner_bytes = inner.to_json_bytes()?;
    let sealed = seal_to_secp256k1_public(&recipient_id.public_key, &inner_bytes)?;
    let ciphertext_hex = hex::encode(&sealed);
    let sign_bytes = envelope_sign_bytes(
        message_id,
        &sender_wire,
        &recipient_norm,
        created_at_ms,
        &ciphertext_hex,
    );
    let signature = sign_delivery_challenge(ident, &sign_bytes)?;
    Ok(json!({
        "ghalbol.share": DELIVERY_MSG_SHARE,
        "format_version": DELIVERY_MSG_FORMAT_VERSION,
        "message_id": message_id,
        "sender_wire": sender_wire,
        "recipient_wire": recipient_norm,
        "created_at_ms": created_at_ms,
        "ciphertext_hex": ciphertext_hex,
        "signature_hex": hex::encode(signature),
    }))
}

pub fn build_attachment_envelope(
    ident: &DecryptedIdentity,
    message_id: &str,
    recipient_wire: &str,
    inner: &crate::attach_v1::AttachmentInner,
    created_at_ms: i64,
) -> Result<Value, String> {
    let sender_wire = ident.identity_wire();
    let recipient_norm = normalize_contact_identity_wire(recipient_wire)?;
    let recipient_id = Identity::parse(&recipient_norm)?;
    if recipient_id.algorithm != crate::identity::IdentityAlgorithm::Secp256k1 {
        return Err("delivery cipher: secp256k1 recipients only in v1".to_string());
    }
    let inner_bytes = inner.to_json_bytes()?;
    let sealed = seal_to_secp256k1_public(&recipient_id.public_key, &inner_bytes)?;
    let ciphertext_hex = hex::encode(&sealed);
    let sign_bytes = envelope_sign_bytes(
        message_id,
        &sender_wire,
        &recipient_norm,
        created_at_ms,
        &ciphertext_hex,
    );
    let signature = sign_delivery_challenge(ident, &sign_bytes)?;
    Ok(json!({
        "ghalbol.share": DELIVERY_MSG_SHARE,
        "format_version": DELIVERY_MSG_FORMAT_VERSION,
        "message_id": message_id,
        "sender_wire": sender_wire,
        "recipient_wire": recipient_norm,
        "created_at_ms": created_at_ms,
        "ciphertext_hex": ciphertext_hex,
        "signature_hex": hex::encode(signature),
    }))
}

pub fn open_text_from_envelope(
    ident: &DecryptedIdentity,
    envelope: &Value,
) -> Result<(String, String, String), String> {
    verify_delivery_envelope(envelope)?;
    let sender_wire = envelope
        .get("sender_wire")
        .and_then(|v| v.as_str())
        .ok_or("missing sender_wire")?
        .to_string();
    let message_id = envelope
        .get("message_id")
        .and_then(|v| v.as_str())
        .ok_or("missing message_id")?
        .to_string();
    let ciphertext_hex = envelope
        .get("ciphertext_hex")
        .and_then(|v| v.as_str())
        .ok_or("missing ciphertext_hex")?;
    let sealed = hex::decode(ciphertext_hex.trim()).map_err(|e| format!("ciphertext hex: {e}"))?;
    let sk = ident.secp256k1_secret();
    let plain = crate::offline_seal_v1::open_sealed_secp256k1(sk, &sealed)?;
    let inner: Value = serde_json::from_slice(&plain).map_err(|e| format!("inner json: {e}"))?;
    let text = inner
        .get("text")
        .and_then(|v| v.as_str())
        .ok_or("inner text missing")?
        .to_string();
    Ok((message_id, sender_wire, text))
}

pub fn open_voice_from_envelope(
    ident: &DecryptedIdentity,
    envelope: &Value,
) -> Result<(String, String, u32, Vec<u8>), String> {
    let (message_id, sender_wire, plain) = open_plain_from_envelope(ident, envelope)?;
    let inner = crate::voice_msg_v1::VoiceInner::from_json_bytes(&plain)?;
    Ok((
        message_id,
        sender_wire,
        inner.duration_ms,
        inner.opus_bytes()?,
    ))
}

pub enum DeliveryOpenedMessage {
    Text {
        message_id: String,
        sender_wire: String,
        text: String,
    },
    Voice {
        message_id: String,
        sender_wire: String,
        duration_ms: u32,
        opus_blob: Vec<u8>,
    },
    /// Full E2E mailbox attachment (file bytes in sealed inner).
    Attachment {
        message_id: String,
        sender_wire: String,
        inner: crate::attach_v1::AttachmentInner,
    },
}

pub fn open_message_from_envelope(
    ident: &DecryptedIdentity,
    envelope: &Value,
) -> Result<DeliveryOpenedMessage, String> {
    let (message_id, sender_wire, plain) = open_plain_from_envelope(ident, envelope)?;
    let inner: Value = serde_json::from_slice(&plain).map_err(|e| format!("inner json: {e}"))?;
    if inner.get("voice_msg_version").is_some() || inner.get("audio_b64").is_some() {
        let voice = crate::voice_msg_v1::VoiceInner::from_json_bytes(&plain)?;
        return Ok(DeliveryOpenedMessage::Voice {
            message_id,
            sender_wire,
            duration_ms: voice.duration_ms,
            opus_blob: voice.opus_bytes()?,
        });
    }
    if crate::attach_v1::AttachmentInner::is_mailbox_payload(&inner)
        || (inner.get("attachment_version").is_some() && inner.get("file_b64").is_some())
    {
        let attach = crate::attach_v1::AttachmentInner::from_json_bytes(&plain)?;
        return Ok(DeliveryOpenedMessage::Attachment {
            message_id,
            sender_wire,
            inner: attach,
        });
    }
    let text = inner
        .get("text")
        .and_then(|v| v.as_str())
        .ok_or("inner text missing")?
        .to_string();
    Ok(DeliveryOpenedMessage::Text {
        message_id,
        sender_wire,
        text,
    })
}

fn open_plain_from_envelope(
    ident: &DecryptedIdentity,
    envelope: &Value,
) -> Result<(String, String, Vec<u8>), String> {
    verify_delivery_envelope(envelope)?;
    let sender_wire = envelope
        .get("sender_wire")
        .and_then(|v| v.as_str())
        .ok_or("missing sender_wire")?
        .to_string();
    let message_id = envelope
        .get("message_id")
        .and_then(|v| v.as_str())
        .ok_or("missing message_id")?
        .to_string();
    let ciphertext_hex = envelope
        .get("ciphertext_hex")
        .and_then(|v| v.as_str())
        .ok_or("missing ciphertext_hex")?;
    let sealed = hex::decode(ciphertext_hex.trim()).map_err(|e| format!("ciphertext hex: {e}"))?;
    let sk = ident.secp256k1_secret();
    let plain = crate::offline_seal_v1::open_sealed_secp256k1(sk, &sealed)?;
    Ok((message_id, sender_wire, plain))
}

pub fn verify_delivery_envelope(envelope: &Value) -> Result<(), String> {
    let share = envelope
        .get("ghalbol.share")
        .and_then(|v| v.as_str())
        .ok_or("missing ghalbol.share")?;
    if share != DELIVERY_MSG_SHARE {
        return Err(format!("unknown share: {share}"));
    }
    let message_id = envelope
        .get("message_id")
        .and_then(|v| v.as_str())
        .ok_or("missing message_id")?;
    let sender_wire = envelope
        .get("sender_wire")
        .and_then(|v| v.as_str())
        .ok_or("missing sender_wire")?;
    let recipient_wire = envelope
        .get("recipient_wire")
        .and_then(|v| v.as_str())
        .ok_or("missing recipient_wire")?;
    let created_at_ms = envelope
        .get("created_at_ms")
        .and_then(|v| v.as_i64())
        .ok_or("missing created_at_ms")?;
    let ciphertext_hex = envelope
        .get("ciphertext_hex")
        .and_then(|v| v.as_str())
        .ok_or("missing ciphertext_hex")?;
    let signature_hex = envelope
        .get("signature_hex")
        .and_then(|v| v.as_str())
        .ok_or("missing signature_hex")?;
    let sig = hex::decode(signature_hex.trim()).map_err(|e| format!("signature hex: {e}"))?;
    let sign_bytes = envelope_sign_bytes(
        message_id,
        sender_wire,
        recipient_wire,
        created_at_ms,
        ciphertext_hex,
    );
    verify_delivery_signature(sender_wire, &sign_bytes, &sig)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_secp256k1() {
        let (_ks_a, a) = crate::create_keystore_v1("pw", None).unwrap();
        let (_ks_b, b) = crate::create_keystore_v1("pw2", None).unwrap();
        let env = build_text_envelope(&a, "msg-1", &b.identity_wire(), "hello", 1_000).unwrap();
        verify_delivery_envelope(&env).unwrap();
        let (_id, _from, text) = open_text_from_envelope(&b, &env).unwrap();
        assert_eq!(text, "hello");
    }
}
