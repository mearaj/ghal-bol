//! Direct-message wire envelope (**`ghal_bol_msg_v2`**) — identity signatures + transport ciphertext.
//!
//! Text bodies use **transport KEM v2** (`DM_CIPHER_TRANSPORT_V2`) after `TransportKemHello`.
//! Identity keys sign envelopes only; payload confidentiality uses X25519 transport keys.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use x25519_dalek::StaticSecret;

use crate::identity::same_contact_identity;
use crate::identity_sign::verify_identity_signature;
use crate::keystore_v1::DecryptedIdentity;
use crate::public_key_util::normalize_contact_identity_wire;
use crate::symmetric_seal::{open_symmetric, seal_symmetric};
use crate::transport_kem_v1::{
    DM_CIPHER_TRANSPORT_V2, derive_dm_transport_message_key, parse_transport_pubkey_hex,
    transport_public_key_bytes,
};

pub const MSG_SHARE: &str = "ghal_bol_msg_v1";
pub const MSG_FORMAT_VERSION: u64 = 2;

/// Optional transport KEM context for outbound DM text (`DM_CIPHER_TRANSPORT_V2`).
pub struct DmSealTransportCtx<'a> {
    pub local_sk: &'a StaticSecret,
    pub peer_pk: &'a [u8; 32],
}

/// Optional transport KEM context for inbound DM text (`DM_CIPHER_TRANSPORT_V2`).
pub struct DmOpenTransportCtx<'a> {
    pub local_sk: &'a StaticSecret,
    pub peer_pk: &'a [u8; 32],
}

fn envelope_recipient_ok(env: &MsgEnvelope, my_identity_wire: &str) -> bool {
    same_contact_identity(env.recipient_public_key_hex.trim(), my_identity_wire)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MsgKind {
    Text,
    /// Opus voice note — same seal/ack rail as text (not call media).
    Voice,
    AckReceived,
    AckRead,
    AckRequest,
    /// Exchange per-node X25519 transport public keys (payload confidentiality KEM).
    TransportKemHello,
    /// Sender-served file offer (control plane only; bytes on `/ghal-bol/attach/1.0.0`).
    AttachmentOffer,
    /// Recipient finished download+verify of an attachment offer (`ref_id` = offer id).
    AttachmentComplete,
    AvailabilityStatus,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MsgEnvelope {
    #[serde(rename = "ghalbol.share")]
    pub wire_share: String,
    pub format_version: u64,
    pub id: String,
    pub kind: MsgKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ref_id: Option<String>,
    pub sender_public_key_hex: String,
    pub recipient_public_key_hex: String,
    pub created_at_ms: i64,
    /// On **`ack_received` only:** when the recipient first accepted the referenced text
    /// (`ref_id`). Recipient authority; must not change on duplicate text retries.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub received_at_ms: Option<i64>,
    pub ciphertext_hex: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature_hex: Option<String>,
    /// Sender's X25519 transport public key (`TransportKemHello` only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transport_x25519_hex: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ParsedText {
    pub id: String,
    pub sender_public_key_hex: String,
    pub created_at_ms: i64,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct ParsedVoice {
    pub id: String,
    pub sender_public_key_hex: String,
    pub created_at_ms: i64,
    pub duration_ms: u32,
    pub opus_blob: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct ParsedAttachmentOffer {
    pub id: String,
    pub sender_public_key_hex: String,
    pub created_at_ms: i64,
    pub blob_id: String,
    pub file_name: String,
    pub mime_type: String,
    pub size_plaintext: u64,
    pub sha256_plaintext: String,
    pub content_key_b64: String,
    pub expires_at_ms: i64,
    /// Set when the sealed DM carries the file (mailbox / LAN inline). Empty → LAN mux fetch.
    pub file_plain: Option<Vec<u8>>,
}

#[derive(Clone, Debug)]
pub struct ParsedAck {
    pub id: String,
    pub ref_id: String,
    pub kind: MsgKind,
    pub sender_public_key_hex: String,
    pub created_at_ms: i64,
    pub received_at_ms: Option<i64>,
}

#[derive(Clone, Debug)]
pub struct ParsedAvailabilityStatus {
    pub id: String,
    pub sender_public_key_hex: String,
    pub created_at_ms: i64,
    pub status: Option<String>,
    pub updated_at_ms: i64,
}

#[derive(Clone, Debug)]
pub enum ParsedMsg {
    Text(ParsedText),
    Voice(ParsedVoice),
    AttachmentOffer(ParsedAttachmentOffer),
    AvailabilityStatus(ParsedAvailabilityStatus),
    Ack(ParsedAck),
    TransportKemHello {
        sender_public_key_hex: String,
        transport_pk: [u8; 32],
    },
}

fn canonical_sign_bytes(env: &MsgEnvelope) -> Result<Vec<u8>, String> {
    let mut clone = env.clone();
    clone.signature_hex = None;
    serde_json::to_vec(&clone).map_err(|e| format!("canonical json: {e}"))
}

pub fn sign_envelope(env: &mut MsgEnvelope, sender: &DecryptedIdentity) -> Result<(), String> {
    let bytes = canonical_sign_bytes(env)?;
    let sig = sender.sign_message(&bytes)?;
    env.signature_hex = Some(hex::encode(sig));
    Ok(())
}

pub fn verify_envelope(env: &MsgEnvelope) -> Result<(), String> {
    if env.wire_share != MSG_SHARE {
        return Err(format!("unknown ghalbol.share: {}", env.wire_share));
    }
    if env.format_version != MSG_FORMAT_VERSION {
        return Err("unsupported format_version".to_string());
    }
    let sig_hex = env
        .signature_hex
        .as_deref()
        .ok_or_else(|| "missing signature_hex".to_string())?;
    let sig = hex::decode(sig_hex.trim()).map_err(|e| format!("signature hex: {e}"))?;
    let bytes = canonical_sign_bytes(env)?;
    verify_identity_signature(env.sender_public_key_hex.trim(), &bytes, &sig)?;
    Ok(())
}

pub fn build_text_envelope(
    id: &str,
    sender: &DecryptedIdentity,
    recipient_identity_wire: &str,
    text: &str,
    created_at_ms: i64,
    transport: DmSealTransportCtx<'_>,
) -> Result<MsgEnvelope, String> {
    let sender_hex = sender.identity_wire();
    let recipient_wire = normalize_contact_identity_wire(recipient_identity_wire)?;
    let inner = serde_json::json!({ "text": text });
    let inner_bytes = serde_json::to_vec(&inner).map_err(|e| format!("inner json: {e}"))?;
    let sealed = seal_dm_ciphertext_outbound(sender, &recipient_wire, &inner_bytes, transport)?;
    let mut env = MsgEnvelope {
        wire_share: MSG_SHARE.to_string(),
        format_version: MSG_FORMAT_VERSION,
        id: id.to_string(),
        kind: MsgKind::Text,
        ref_id: None,
        sender_public_key_hex: sender_hex,
        recipient_public_key_hex: recipient_wire,
        created_at_ms,
        received_at_ms: None,
        ciphertext_hex: hex::encode(sealed),
        signature_hex: None,
        transport_x25519_hex: None,
    };
    sign_envelope(&mut env, sender)?;
    Ok(env)
}

/// Build a sealed voice-note envelope (Opus blob in inner JSON). Same transport KEM as text.
pub fn build_voice_envelope(
    id: &str,
    sender: &DecryptedIdentity,
    recipient_identity_wire: &str,
    duration_ms: u32,
    opus_blob: &[u8],
    created_at_ms: i64,
    transport: DmSealTransportCtx<'_>,
) -> Result<MsgEnvelope, String> {
    let sender_hex = sender.identity_wire();
    let recipient_wire = normalize_contact_identity_wire(recipient_identity_wire)?;
    let inner = crate::voice_msg_v1::build_voice_inner(duration_ms, opus_blob)?;
    let inner_bytes = inner.to_json_bytes()?;
    let sealed = seal_dm_ciphertext_outbound(sender, &recipient_wire, &inner_bytes, transport)?;
    let mut env = MsgEnvelope {
        wire_share: MSG_SHARE.to_string(),
        format_version: MSG_FORMAT_VERSION,
        id: id.to_string(),
        kind: MsgKind::Voice,
        ref_id: None,
        sender_public_key_hex: sender_hex,
        recipient_public_key_hex: recipient_wire,
        created_at_ms,
        received_at_ms: None,
        ciphertext_hex: hex::encode(sealed),
        signature_hex: None,
        transport_x25519_hex: None,
    };
    sign_envelope(&mut env, sender)?;
    Ok(env)
}

/// Build a sealed attachment envelope (mailbox file bytes or LAN mux offer JSON).
pub fn build_attachment_offer_envelope(
    id: &str,
    sender: &DecryptedIdentity,
    recipient_identity_wire: &str,
    offer: &Value,
    created_at_ms: i64,
    transport: DmSealTransportCtx<'_>,
) -> Result<MsgEnvelope, String> {
    let sender_hex = sender.identity_wire();
    let recipient_wire = normalize_contact_identity_wire(recipient_identity_wire)?;
    let inner_bytes = serde_json::to_vec(offer).map_err(|e| format!("offer json: {e}"))?;
    let sealed = seal_dm_ciphertext_outbound(sender, &recipient_wire, &inner_bytes, transport)?;
    let mut env = MsgEnvelope {
        wire_share: MSG_SHARE.to_string(),
        format_version: MSG_FORMAT_VERSION,
        id: id.to_string(),
        kind: MsgKind::AttachmentOffer,
        ref_id: None,
        sender_public_key_hex: sender_hex,
        recipient_public_key_hex: recipient_wire,
        created_at_ms,
        received_at_ms: None,
        ciphertext_hex: hex::encode(sealed),
        signature_hex: None,
        transport_x25519_hex: None,
    };
    sign_envelope(&mut env, sender)?;
    Ok(env)
}

pub fn build_availability_status_envelope(
    id: &str,
    sender: &DecryptedIdentity,
    recipient_identity_wire: &str,
    status: Option<&str>,
    updated_at_ms: i64,
    created_at_ms: i64,
    transport: DmSealTransportCtx<'_>,
) -> Result<MsgEnvelope, String> {
    let sender_hex = sender.identity_wire();
    let recipient_wire = normalize_contact_identity_wire(recipient_identity_wire)?;
    let inner = serde_json::json!({
        "status": status.unwrap_or(""),
        "updated_at_ms": updated_at_ms,
    });
    let inner_bytes = serde_json::to_vec(&inner).map_err(|e| format!("status json: {e}"))?;
    let sealed = seal_dm_ciphertext_outbound(sender, &recipient_wire, &inner_bytes, transport)?;
    let mut env = MsgEnvelope {
        wire_share: MSG_SHARE.to_string(),
        format_version: MSG_FORMAT_VERSION,
        id: id.to_string(),
        kind: MsgKind::AvailabilityStatus,
        ref_id: None,
        sender_public_key_hex: sender_hex,
        recipient_public_key_hex: recipient_wire,
        created_at_ms,
        received_at_ms: None,
        ciphertext_hex: hex::encode(sealed),
        signature_hex: None,
        transport_x25519_hex: None,
    };
    sign_envelope(&mut env, sender)?;
    Ok(env)
}

/// Signed envelope advertising this node's X25519 transport public key to a contact.
pub fn build_transport_kem_hello_envelope(
    id: &str,
    sender: &DecryptedIdentity,
    recipient_identity_wire: &str,
    local_transport_sk: &StaticSecret,
    created_at_ms: i64,
) -> Result<MsgEnvelope, String> {
    let sender_hex = sender.identity_wire();
    let recipient_wire = normalize_contact_identity_wire(recipient_identity_wire)?;
    let pk = transport_public_key_bytes(local_transport_sk);
    let mut env = MsgEnvelope {
        wire_share: MSG_SHARE.to_string(),
        format_version: MSG_FORMAT_VERSION,
        id: id.to_string(),
        kind: MsgKind::TransportKemHello,
        ref_id: None,
        sender_public_key_hex: sender_hex,
        recipient_public_key_hex: recipient_wire,
        created_at_ms,
        received_at_ms: None,
        ciphertext_hex: String::new(),
        signature_hex: None,
        transport_x25519_hex: Some(hex::encode(pk)),
    };
    sign_envelope(&mut env, sender)?;
    Ok(env)
}

pub fn build_ack_envelope(
    id: &str,
    ref_id: &str,
    kind: MsgKind,
    sender: &DecryptedIdentity,
    recipient_public_key_hex: &str,
    created_at_ms: i64,
    received_at_ms: Option<i64>,
) -> Result<MsgEnvelope, String> {
    if kind != MsgKind::AckReceived
        && kind != MsgKind::AckRead
        && kind != MsgKind::AckRequest
        && kind != MsgKind::AttachmentComplete
    {
        return Err("build_ack_envelope: kind must be ack or attachment_complete".to_string());
    }
    let sender_hex = sender.identity_wire();
    let mut env = MsgEnvelope {
        wire_share: MSG_SHARE.to_string(),
        format_version: MSG_FORMAT_VERSION,
        id: id.to_string(),
        kind,
        ref_id: Some(ref_id.to_string()),
        sender_public_key_hex: sender_hex,
        recipient_public_key_hex: normalize_contact_identity_wire(recipient_public_key_hex)?,
        created_at_ms,
        received_at_ms: if kind == MsgKind::AckReceived {
            received_at_ms.filter(|t| *t > 0)
        } else {
            None
        },
        ciphertext_hex: String::new(),
        signature_hex: None,
        transport_x25519_hex: None,
    };
    sign_envelope(&mut env, sender)?;
    Ok(env)
}

pub fn parse_envelope_with_transport(
    env: &MsgEnvelope,
    local: &DecryptedIdentity,
    transport: Option<DmOpenTransportCtx<'_>>,
) -> Result<ParsedMsg, String> {
    verify_envelope(env)?;
    if !envelope_recipient_ok(env, &local.identity_wire()) {
        return Err("envelope not addressed to this identity".to_string());
    }
    match env.kind {
        MsgKind::Text => {
            if env.ciphertext_hex.is_empty() {
                return Err("text envelope missing ciphertext".to_string());
            }
            let sealed = hex::decode(env.ciphertext_hex.trim())
                .map_err(|e| format!("ciphertext hex: {e}"))?;
            let plain = open_dm_ciphertext(
                env.sender_public_key_hex.trim(),
                &sealed,
                transport.ok_or_else(|| "transport kem required to decrypt dm text".to_string())?,
                &local.identity_wire(),
            )?;
            let v: Value =
                serde_json::from_slice(&plain).map_err(|e| format!("inner json: {e}"))?;
            let text = v
                .get("text")
                .and_then(|t| t.as_str())
                .ok_or_else(|| "inner json missing text".to_string())?
                .to_string();
            Ok(ParsedMsg::Text(ParsedText {
                id: env.id.clone(),
                sender_public_key_hex: env.sender_public_key_hex.trim().to_string(),
                created_at_ms: env.created_at_ms,
                text,
            }))
        }
        MsgKind::Voice => {
            if env.ciphertext_hex.is_empty() {
                return Err("voice envelope missing ciphertext".to_string());
            }
            let sealed = hex::decode(env.ciphertext_hex.trim())
                .map_err(|e| format!("ciphertext hex: {e}"))?;
            let plain = open_dm_ciphertext(
                env.sender_public_key_hex.trim(),
                &sealed,
                transport
                    .ok_or_else(|| "transport kem required to decrypt dm voice".to_string())?,
                &local.identity_wire(),
            )?;
            let inner = crate::voice_msg_v1::VoiceInner::from_json_bytes(&plain)?;
            Ok(ParsedMsg::Voice(ParsedVoice {
                id: env.id.clone(),
                sender_public_key_hex: env.sender_public_key_hex.trim().to_string(),
                created_at_ms: env.created_at_ms,
                duration_ms: inner.duration_ms,
                opus_blob: inner.opus_bytes()?,
            }))
        }
        MsgKind::AttachmentOffer => {
            if env.ciphertext_hex.is_empty() {
                return Err("attachment_offer missing ciphertext".to_string());
            }
            let sealed = hex::decode(env.ciphertext_hex.trim())
                .map_err(|e| format!("ciphertext hex: {e}"))?;
            let plain = open_dm_ciphertext(
                env.sender_public_key_hex.trim(),
                &sealed,
                transport.ok_or_else(|| {
                    "transport kem required to decrypt attachment_offer".to_string()
                })?,
                &local.identity_wire(),
            )?;
            let v: Value =
                serde_json::from_slice(&plain).map_err(|e| format!("offer json: {e}"))?;
            if crate::attach_v1::AttachmentInner::is_mailbox_payload(&v) {
                let inner = crate::attach_v1::AttachmentInner::from_json_bytes(&plain)?;
                let file_plain = inner.file_bytes()?;
                return Ok(ParsedMsg::AttachmentOffer(ParsedAttachmentOffer {
                    id: env.id.clone(),
                    sender_public_key_hex: env.sender_public_key_hex.trim().to_string(),
                    created_at_ms: env.created_at_ms,
                    blob_id: env.id.clone(),
                    file_name: inner.file_name,
                    mime_type: inner.mime_type,
                    size_plaintext: inner.size_plaintext,
                    sha256_plaintext: inner.sha256_plaintext,
                    content_key_b64: String::new(),
                    expires_at_ms: 0,
                    file_plain: Some(file_plain),
                }));
            }
            Ok(ParsedMsg::AttachmentOffer(ParsedAttachmentOffer {
                id: env.id.clone(),
                sender_public_key_hex: env.sender_public_key_hex.trim().to_string(),
                created_at_ms: env.created_at_ms,
                blob_id: v
                    .get("blob_id")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
                file_name: v
                    .get("file_name")
                    .and_then(|x| x.as_str())
                    .unwrap_or("file")
                    .to_string(),
                mime_type: v
                    .get("mime_type")
                    .and_then(|x| x.as_str())
                    .unwrap_or("application/octet-stream")
                    .to_string(),
                size_plaintext: v
                    .get("size_plaintext")
                    .and_then(|x| x.as_u64())
                    .unwrap_or(0),
                sha256_plaintext: v
                    .get("sha256_plaintext")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
                content_key_b64: v
                    .get("content_key_b64")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
                expires_at_ms: v.get("expires_at_ms").and_then(|x| x.as_i64()).unwrap_or(0),
                file_plain: None,
            }))
        }
        MsgKind::AvailabilityStatus => {
            if env.ciphertext_hex.is_empty() {
                return Err("availability_status missing ciphertext".to_string());
            }
            let sealed = hex::decode(env.ciphertext_hex.trim())
                .map_err(|e| format!("ciphertext hex: {e}"))?;
            let plain = open_dm_ciphertext(
                env.sender_public_key_hex.trim(),
                &sealed,
                transport.ok_or_else(|| {
                    "transport kem required to decrypt availability_status".to_string()
                })?,
                &local.identity_wire(),
            )?;
            let v: Value =
                serde_json::from_slice(&plain).map_err(|e| format!("status json: {e}"))?;
            let status = v
                .get("status")
                .and_then(|x| x.as_str())
                .and_then(crate::preferences_v1::sanitize_peer_display_alias);
            let updated_at_ms = v
                .get("updated_at_ms")
                .and_then(|x| x.as_i64())
                .filter(|t| *t > 0)
                .unwrap_or(env.created_at_ms);
            Ok(ParsedMsg::AvailabilityStatus(ParsedAvailabilityStatus {
                id: env.id.clone(),
                sender_public_key_hex: env.sender_public_key_hex.trim().to_string(),
                created_at_ms: env.created_at_ms,
                status,
                updated_at_ms,
            }))
        }
        MsgKind::AckReceived
        | MsgKind::AckRead
        | MsgKind::AckRequest
        | MsgKind::AttachmentComplete => {
            let ref_id = env
                .ref_id
                .clone()
                .ok_or_else(|| "ack missing ref_id".to_string())?;
            Ok(ParsedMsg::Ack(ParsedAck {
                id: env.id.clone(),
                ref_id,
                kind: env.kind,
                sender_public_key_hex: env.sender_public_key_hex.trim().to_string(),
                created_at_ms: env.created_at_ms,
                received_at_ms: env.received_at_ms.filter(|t| *t > 0),
            }))
        }
        MsgKind::TransportKemHello => {
            let pk_hex = env
                .transport_x25519_hex
                .as_deref()
                .ok_or_else(|| "transport kem hello missing transport_x25519_hex".to_string())?;
            let transport_pk = parse_transport_pubkey_hex(pk_hex)?;
            Ok(ParsedMsg::TransportKemHello {
                sender_public_key_hex: env.sender_public_key_hex.trim().to_string(),
                transport_pk,
            })
        }
    }
}

fn seal_dm_ciphertext_outbound(
    sender: &DecryptedIdentity,
    recipient_wire: &str,
    inner_bytes: &[u8],
    transport: DmSealTransportCtx<'_>,
) -> Result<Vec<u8>, String> {
    let sender_wire = sender.identity_wire();
    let key = derive_dm_transport_message_key(
        transport.local_sk,
        transport.peer_pk,
        &sender_wire,
        recipient_wire,
    )?;
    let sym = seal_symmetric(&key, inner_bytes)?;
    let mut sealed = Vec::with_capacity(1 + sym.len());
    sealed.push(DM_CIPHER_TRANSPORT_V2);
    sealed.extend_from_slice(&sym);
    Ok(sealed)
}

fn open_dm_ciphertext(
    sender_identity_wire: &str,
    sealed: &[u8],
    transport: DmOpenTransportCtx<'_>,
    local_identity_wire: &str,
) -> Result<Vec<u8>, String> {
    if sealed.first() != Some(&DM_CIPHER_TRANSPORT_V2) {
        return Err("dm: expected transport kem v2 ciphertext prefix".to_string());
    }
    let key = derive_dm_transport_message_key(
        transport.local_sk,
        transport.peer_pk,
        local_identity_wire,
        sender_identity_wire,
    )?;
    open_symmetric(&key, &sealed[1..])
}

pub fn envelope_to_frame_bytes(env: &MsgEnvelope) -> Result<Vec<u8>, String> {
    let json = serde_json::to_vec(env).map_err(|e| format!("encode envelope: {e}"))?;
    let len = u32::try_from(json.len()).map_err(|_| "envelope too large".to_string())?;
    let mut out = Vec::with_capacity(4 + json.len());
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&json);
    Ok(out)
}

pub fn frame_bytes_to_envelope(frame: &[u8]) -> Result<MsgEnvelope, String> {
    if frame.len() < 4 {
        return Err("frame too short".to_string());
    }
    let len = u32::from_le_bytes([frame[0], frame[1], frame[2], frame[3]]) as usize;
    if frame.len() < 4 + len {
        return Err("frame truncated".to_string());
    }
    let body = &frame[4..4 + len];
    serde_json::from_slice(body).map_err(|e| format!("decode envelope: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::create_keystore_v1;
    use crate::transport_kem_v1::generate_transport_keypair;

    fn transport_pair() -> (
        x25519_dalek::StaticSecret,
        [u8; 32],
        x25519_dalek::StaticSecret,
        [u8; 32],
    ) {
        let (sk_a, pk_a) = generate_transport_keypair();
        let (sk_b, pk_b) = generate_transport_keypair();
        (sk_a, pk_a, sk_b, pk_b)
    }

    #[test]
    fn text_roundtrip_sign_and_open() {
        let (_ks_a, alice) = create_keystore_v1("pw", None).unwrap();
        let (_ks_b, bob) = create_keystore_v1("pw2", None).unwrap();
        let (sk_a, pk_a, sk_b, pk_b) = transport_pair();
        let env = build_text_envelope(
            "msg-1",
            &alice,
            &bob.identity_wire(),
            "hello",
            1_700_000_000_000,
            DmSealTransportCtx {
                local_sk: &sk_a,
                peer_pk: &pk_b,
            },
        )
        .unwrap();
        let parsed = parse_envelope_with_transport(
            &env,
            &bob,
            Some(DmOpenTransportCtx {
                local_sk: &sk_b,
                peer_pk: &pk_a,
            }),
        )
        .unwrap();
        match parsed {
            ParsedMsg::Text(t) => assert_eq!(t.text, "hello"),
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn voice_roundtrip_sign_and_open() {
        let (_ks_a, alice) = create_keystore_v1("pw", None).unwrap();
        let (_ks_b, bob) = create_keystore_v1("pw2", None).unwrap();
        let (sk_a, pk_a, sk_b, pk_b) = transport_pair();
        let opus = vec![1, 2, 3, 4, 5];
        let env = build_voice_envelope(
            "voice-1",
            &alice,
            &bob.identity_wire(),
            1_234,
            &opus,
            1_700_000_000_000,
            DmSealTransportCtx {
                local_sk: &sk_a,
                peer_pk: &pk_b,
            },
        )
        .unwrap();
        let parsed = parse_envelope_with_transport(
            &env,
            &bob,
            Some(DmOpenTransportCtx {
                local_sk: &sk_b,
                peer_pk: &pk_a,
            }),
        )
        .unwrap();
        match parsed {
            ParsedMsg::Voice(v) => {
                assert_eq!(v.duration_ms, 1_234);
                assert_eq!(v.opus_blob, opus);
            }
            _ => panic!("expected voice"),
        }
    }

    #[test]
    fn ack_received_includes_received_at_ms_in_signature() {
        let (_ks_a, alice) = create_keystore_v1("pw", None).unwrap();
        let (_ks_b, bob) = create_keystore_v1("pw2", None).unwrap();
        let env = build_ack_envelope(
            "ack-1",
            "msg-1",
            MsgKind::AckReceived,
            &bob,
            &alice.identity_wire(),
            1_700_000_000_100,
            Some(1_700_000_000_000),
        )
        .unwrap();
        verify_envelope(&env).unwrap();
        let parsed = parse_envelope_with_transport(&env, &alice, None).unwrap();
        match parsed {
            ParsedMsg::Ack(a) => {
                assert_eq!(a.ref_id, "msg-1");
                assert_eq!(a.received_at_ms, Some(1_700_000_000_000));
            }
            _ => panic!("expected ack"),
        }
    }

    #[test]
    fn transport_kem_v2_roundtrip_after_hello() {
        use crate::transport_kem_v1::{DM_CIPHER_TRANSPORT_V2, generate_transport_keypair};

        let (_ks_a, alice) = create_keystore_v1("pw", None).unwrap();
        let (_ks_b, bob) = create_keystore_v1("pw2", None).unwrap();
        let (sk_a, pk_a) = generate_transport_keypair();
        let (sk_b, pk_b) = generate_transport_keypair();

        let hello =
            build_transport_kem_hello_envelope("tkem-1", &alice, &bob.identity_wire(), &sk_a, 1)
                .unwrap();
        let parsed_hello = parse_envelope_with_transport(&hello, &bob, None).unwrap();
        match parsed_hello {
            ParsedMsg::TransportKemHello { transport_pk, .. } => assert_eq!(transport_pk, pk_a),
            _ => panic!("expected transport hello"),
        }

        let open_b = DmOpenTransportCtx {
            local_sk: &sk_b,
            peer_pk: &pk_a,
        };

        let env = build_text_envelope(
            "tv2-1",
            &alice,
            &bob.identity_wire(),
            "transport-v2",
            3,
            DmSealTransportCtx {
                local_sk: &sk_a,
                peer_pk: &pk_b,
            },
        )
        .unwrap();
        let sealed = hex::decode(&env.ciphertext_hex).unwrap();
        assert_eq!(sealed.first(), Some(&DM_CIPHER_TRANSPORT_V2));
        let parsed = parse_envelope_with_transport(&env, &bob, Some(open_b)).unwrap();
        match parsed {
            ParsedMsg::Text(t) => assert_eq!(t.text, "transport-v2"),
            _ => panic!("expected text"),
        }
    }
}
