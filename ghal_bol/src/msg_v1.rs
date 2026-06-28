//! Direct-message wire envelope (**`ghal_bol_msg_v2`**) — single secp256k1 identity per device.
//!
//! One public key: libp2p PeerId, envelope signatures, and ciphertext sealing.
//!
//! **Delivery sync (`dm_delivery_sync.dart`):** recipient sends `ack_received` / `ack_read`;
//! sender resends text until acked. `ack_request` is not used on the wire.

use libp2p_identity::Keypair;
use secp256k1::SecretKey;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::public_key_util::secp256k1_public_key_from_hex;
use crate::secp256k1_seal::{open_sealed_secp256k1, seal_to_secp256k1_public};

pub const MSG_SHARE: &str = "ghal_bol_msg_v1";
pub const MSG_FORMAT_VERSION: u64 = 2;
pub const STREAM_PROTOCOL: &str = "/ghal-bol/msg/1.0.0";

fn envelope_recipient_ok(env: &MsgEnvelope, my_public_key_hex: &str) -> bool {
    env.recipient_public_key_hex.trim() == my_public_key_hex.trim()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MsgKind {
    Text,
    AckReceived,
    AckRead,
    AckRequest,
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
}

#[derive(Clone, Debug)]
pub struct ParsedText {
    pub id: String,
    pub sender_public_key_hex: String,
    pub created_at_ms: i64,
    pub text: String,
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
pub enum ParsedMsg {
    Text(ParsedText),
    Ack(ParsedAck),
}

fn canonical_sign_bytes(env: &MsgEnvelope) -> Result<Vec<u8>, String> {
    let mut clone = env.clone();
    clone.signature_hex = None;
    serde_json::to_vec(&clone).map_err(|e| format!("canonical json: {e}"))
}

pub fn sign_envelope(env: &mut MsgEnvelope, sender: &Keypair) -> Result<(), String> {
    let bytes = canonical_sign_bytes(env)?;
    let sig = sender.sign(&bytes).map_err(|e| format!("sign: {e}"))?;
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
    let sender_pk = secp256k1_public_key_from_hex(env.sender_public_key_hex.trim())?;
    let libp2p_pk = libp2p_identity::PublicKey::from(sender_pk);
    let bytes = canonical_sign_bytes(env)?;
    if !libp2p_pk.verify(&bytes, &sig) {
        return Err("signature verification failed".to_string());
    }
    Ok(())
}

pub fn build_text_envelope(
    id: &str,
    sender: &Keypair,
    recipient_public_key_hex: &str,
    text: &str,
    created_at_ms: i64,
) -> Result<MsgEnvelope, String> {
    let sender_pk = sender
        .public()
        .try_into_secp256k1()
        .map_err(|e| format!("sender key: {e}"))?;
    let sender_hex = crate::public_key_util::secp256k1_public_key_to_hex(&sender_pk);
    let recipient = recipient_public_key_hex.trim();
    let recipient_pk = secp256k1_public_key_from_hex(recipient)?;
    let inner = serde_json::json!({ "text": text });
    let inner_bytes = serde_json::to_vec(&inner).map_err(|e| format!("inner json: {e}"))?;
    let sealed = seal_to_secp256k1_public(&recipient_pk.to_bytes(), &inner_bytes)?;
    let mut env = MsgEnvelope {
        wire_share: MSG_SHARE.to_string(),
        format_version: MSG_FORMAT_VERSION,
        id: id.to_string(),
        kind: MsgKind::Text,
        ref_id: None,
        sender_public_key_hex: sender_hex,
        recipient_public_key_hex: recipient.to_string(),
        created_at_ms,
        received_at_ms: None,
        ciphertext_hex: hex::encode(sealed),
        signature_hex: None,
    };
    sign_envelope(&mut env, sender)?;
    Ok(env)
}

pub fn build_ack_envelope(
    id: &str,
    ref_id: &str,
    kind: MsgKind,
    sender: &Keypair,
    recipient_public_key_hex: &str,
    created_at_ms: i64,
    received_at_ms: Option<i64>,
) -> Result<MsgEnvelope, String> {
    if kind != MsgKind::AckReceived && kind != MsgKind::AckRead && kind != MsgKind::AckRequest {
        return Err("build_ack_envelope: kind must be ack".to_string());
    }
    let sender_pk = sender
        .public()
        .try_into_secp256k1()
        .map_err(|e| format!("sender key: {e}"))?;
    let sender_hex = crate::public_key_util::secp256k1_public_key_to_hex(&sender_pk);
    let mut env = MsgEnvelope {
        wire_share: MSG_SHARE.to_string(),
        format_version: MSG_FORMAT_VERSION,
        id: id.to_string(),
        kind,
        ref_id: Some(ref_id.to_string()),
        sender_public_key_hex: sender_hex,
        recipient_public_key_hex: recipient_public_key_hex.trim().to_string(),
        created_at_ms,
        received_at_ms: if kind == MsgKind::AckReceived {
            received_at_ms.filter(|t| *t > 0)
        } else {
            None
        },
        ciphertext_hex: String::new(),
        signature_hex: None,
    };
    sign_envelope(&mut env, sender)?;
    Ok(env)
}

pub fn parse_envelope(
    env: &MsgEnvelope,
    my_public_key_hex: &str,
    my_secret: &SecretKey,
) -> Result<ParsedMsg, String> {
    verify_envelope(env)?;
    if !envelope_recipient_ok(env, my_public_key_hex) {
        return Err("envelope not addressed to this identity".to_string());
    }
    match env.kind {
        MsgKind::Text => {
            if env.ciphertext_hex.is_empty() {
                return Err("text envelope missing ciphertext".to_string());
            }
            let sealed = hex::decode(env.ciphertext_hex.trim())
                .map_err(|e| format!("ciphertext hex: {e}"))?;
            let plain = open_sealed_secp256k1(my_secret, &sealed)?;
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
        MsgKind::AckReceived | MsgKind::AckRead | MsgKind::AckRequest => {
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
    }
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

    #[test]
    fn text_roundtrip_sign_and_open() {
        let (_ks_a, alice) = create_keystore_v1("pw", None).unwrap();
        let (_ks_b, bob) = create_keystore_v1("pw2", None).unwrap();
        let env = build_text_envelope(
            "msg-1",
            alice.keypair(),
            &bob.public_key_hex(),
            "hello",
            1_700_000_000_000,
        )
        .unwrap();
        let parsed = parse_envelope(&env, &bob.public_key_hex(), bob.secp256k1_secret()).unwrap();
        match parsed {
            ParsedMsg::Text(t) => assert_eq!(t.text, "hello"),
            _ => panic!("expected text"),
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
            bob.keypair(),
            &alice.public_key_hex(),
            1_700_000_000_100,
            Some(1_700_000_000_000),
        )
        .unwrap();
        verify_envelope(&env).unwrap();
        let parsed = parse_envelope(&env, &alice.public_key_hex(), alice.secp256k1_secret()).unwrap();
        match parsed {
            ParsedMsg::Ack(a) => {
                assert_eq!(a.ref_id, "msg-1");
                assert_eq!(a.received_at_ms, Some(1_700_000_000_000));
            }
            _ => panic!("expected ack"),
        }
    }
}
