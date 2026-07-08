//! Voice-call **signaling** envelopes on the DM stream (`ghal_bol_call_v1`).
//!
//! Payload ciphertext uses **transport KEM v2** (`CALL_CIPHER_TRANSPORT_V2`) after
//! `TransportKemHello` on the DM stream. Identity keys sign envelopes only.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use x25519_dalek::StaticSecret;

use crate::identity::same_contact_identity;
use crate::identity_sign::verify_identity_signature;
use crate::keystore_v1::DecryptedIdentity;
use crate::public_key_util::normalize_contact_identity_wire;
use crate::symmetric_seal::{open_symmetric, seal_symmetric};
use crate::transport_kem_v1::{
    CALL_CIPHER_TRANSPORT_V2, derive_call_sig_transport_message_key,
};

pub const CALL_SHARE: &str = "ghal_bol_call_v1";
pub const CALL_FORMAT_VERSION: u64 = 1;

/// Transport KEM context for outbound call signaling.
pub struct CallSealTransportCtx<'a> {
    pub local_sk: &'a StaticSecret,
    pub peer_pk: &'a [u8; 32],
}

/// Transport KEM context for inbound call signaling.
pub struct CallOpenTransportCtx<'a> {
    pub local_sk: &'a StaticSecret,
    pub peer_pk: &'a [u8; 32],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallSigKind {
    Invite,
    Accept,
    Reject,
    Hangup,
    SdpOffer,
    SdpAnswer,
    Ice,
    VideoOn,
    VideoOff,
}

impl CallSigKind {
    pub fn parse_wire(s: &str) -> Result<Self, String> {
        match s.trim() {
            "invite" => Ok(Self::Invite),
            "accept" => Ok(Self::Accept),
            "reject" => Ok(Self::Reject),
            "hangup" => Ok(Self::Hangup),
            "sdp_offer" => Ok(Self::SdpOffer),
            "sdp_answer" => Ok(Self::SdpAnswer),
            "ice" => Ok(Self::Ice),
            "video_on" => Ok(Self::VideoOn),
            "video_off" => Ok(Self::VideoOff),
            other => Err(format!("unknown call signal: {other}")),
        }
    }

    pub fn wire_name(self) -> &'static str {
        match self {
            Self::Invite => "invite",
            Self::Accept => "accept",
            Self::Reject => "reject",
            Self::Hangup => "hangup",
            Self::SdpOffer => "sdp_offer",
            Self::SdpAnswer => "sdp_answer",
            Self::Ice => "ice",
            Self::VideoOn => "video_on",
            Self::VideoOff => "video_off",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CallSigEnvelope {
    #[serde(rename = "ghalbol.share")]
    pub wire_share: String,
    pub format_version: u64,
    pub id: String,
    pub kind: CallSigKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ref_id: Option<String>,
    pub sender_public_key_hex: String,
    pub recipient_public_key_hex: String,
    pub created_at_ms: i64,
    pub ciphertext_hex: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature_hex: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ParsedCallSignal {
    pub id: String,
    pub call_id: String,
    pub kind: CallSigKind,
    pub sender_public_key_hex: String,
    pub created_at_ms: i64,
    pub payload: Value,
}

fn envelope_recipient_ok(env: &CallSigEnvelope, my_identity_wire: &str) -> bool {
    same_contact_identity(env.recipient_public_key_hex.trim(), my_identity_wire)
}

fn canonical_sign_bytes(env: &CallSigEnvelope) -> Result<Vec<u8>, String> {
    let mut clone = env.clone();
    clone.signature_hex = None;
    serde_json::to_vec(&clone).map_err(|e| format!("canonical json: {e}"))
}

pub fn sign_call_envelope(env: &mut CallSigEnvelope, sender: &DecryptedIdentity) -> Result<(), String> {
    let bytes = canonical_sign_bytes(env)?;
    let sig = sender.sign_message(&bytes)?;
    env.signature_hex = Some(hex::encode(sig));
    Ok(())
}

pub fn verify_call_envelope(env: &CallSigEnvelope) -> Result<(), String> {
    if env.wire_share != CALL_SHARE {
        return Err(format!("unknown ghalbol.share: {}", env.wire_share));
    }
    if env.format_version != CALL_FORMAT_VERSION {
        return Err("unsupported call format_version".to_string());
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

pub fn call_envelope_from_frame(frame: &[u8]) -> Result<CallSigEnvelope, String> {
    if frame.len() < 4 {
        return Err("frame too short".to_string());
    }
    let len = u32::from_le_bytes([frame[0], frame[1], frame[2], frame[3]]) as usize;
    if frame.len() < 4 + len {
        return Err("frame truncated".to_string());
    }
    let body = &frame[4..4 + len];
    serde_json::from_slice(body).map_err(|e| format!("decode call envelope: {e}"))
}

pub fn frame_wire_share(frame: &[u8]) -> Result<String, String> {
    if frame.len() < 4 {
        return Err("frame too short".to_string());
    }
    let len = u32::from_le_bytes([frame[0], frame[1], frame[2], frame[3]]) as usize;
    if frame.len() < 4 + len {
        return Err("frame truncated".to_string());
    }
    let body = &frame[4..4 + len];
    let v: Value = serde_json::from_slice(body).map_err(|e| format!("frame json: {e}"))?;
    v.get("ghalbol.share")
        .and_then(|x| x.as_str())
        .map(str::to_string)
        .ok_or_else(|| "missing ghalbol.share".to_string())
}

pub fn build_call_envelope(
    id: &str,
    call_id: &str,
    kind: CallSigKind,
    sender: &DecryptedIdentity,
    recipient_identity_wire: &str,
    payload: Value,
    created_at_ms: i64,
    transport: CallSealTransportCtx<'_>,
) -> Result<CallSigEnvelope, String> {
    let sender_wire = sender.identity_wire();
    let recipient_wire = normalize_contact_identity_wire(recipient_identity_wire)?;
    let inner_bytes = serde_json::to_vec(&payload).unwrap_or_else(|_| b"{}".to_vec());
    let sealed = seal_call_payload_outbound(sender, &recipient_wire, &inner_bytes, transport)?;
    let mut env = CallSigEnvelope {
        wire_share: CALL_SHARE.to_string(),
        format_version: CALL_FORMAT_VERSION,
        id: id.to_string(),
        kind,
        ref_id: Some(call_id.to_string()),
        sender_public_key_hex: sender_wire,
        recipient_public_key_hex: recipient_wire,
        created_at_ms,
        ciphertext_hex: hex::encode(sealed),
        signature_hex: None,
    };
    sign_call_envelope(&mut env, sender)?;
    Ok(env)
}

pub fn parse_call_envelope_with_transport(
    env: &CallSigEnvelope,
    local: &DecryptedIdentity,
    transport: Option<CallOpenTransportCtx<'_>>,
) -> Result<ParsedCallSignal, String> {
    verify_call_envelope(env)?;
    if !envelope_recipient_ok(env, &local.identity_wire()) {
        return Err("call envelope not addressed to this identity".to_string());
    }
    let call_id = env
        .ref_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "call envelope missing ref_id (call_id)".to_string())?
        .to_string();
    let payload = if env.ciphertext_hex.trim().is_empty() {
        Value::Object(Default::default())
    } else {
        let sealed =
            hex::decode(env.ciphertext_hex.trim()).map_err(|e| format!("ciphertext hex: {e}"))?;
        let plain = open_call_payload_ciphertext(
            local,
            env.sender_public_key_hex.trim(),
            &sealed,
            transport,
        )?;
        if plain.is_empty() {
            Value::Object(Default::default())
        } else {
            serde_json::from_slice(&plain).map_err(|e| format!("inner json: {e}"))?
        }
    };
    Ok(ParsedCallSignal {
        id: env.id.clone(),
        call_id,
        kind: env.kind,
        sender_public_key_hex: env.sender_public_key_hex.trim().to_string(),
        created_at_ms: env.created_at_ms,
        payload,
    })
}

fn seal_call_payload_outbound(
    sender: &DecryptedIdentity,
    recipient_wire: &str,
    inner_bytes: &[u8],
    transport: CallSealTransportCtx<'_>,
) -> Result<Vec<u8>, String> {
    if inner_bytes == b"{}" {
        return Ok(Vec::new());
    }
    let key = derive_call_sig_transport_message_key(
        transport.local_sk,
        transport.peer_pk,
        &sender.identity_wire(),
        recipient_wire,
    )?;
    let sym = seal_symmetric(&key, inner_bytes)?;
    let mut sealed = Vec::with_capacity(1 + sym.len());
    sealed.push(CALL_CIPHER_TRANSPORT_V2);
    sealed.extend_from_slice(&sym);
    Ok(sealed)
}

fn open_call_payload_ciphertext(
    local: &DecryptedIdentity,
    sender_identity_wire: &str,
    sealed: &[u8],
    transport: Option<CallOpenTransportCtx<'_>>,
) -> Result<Vec<u8>, String> {
    if sealed.is_empty() {
        return Ok(Vec::new());
    }
    if sealed.first() != Some(&CALL_CIPHER_TRANSPORT_V2) {
        return Err("call ciphertext: unsupported cipher prefix".to_string());
    }
    let transport = transport.ok_or_else(|| {
        "call decrypt: transport kem context required for ciphertext".to_string()
    })?;
    let key = derive_call_sig_transport_message_key(
        transport.local_sk,
        transport.peer_pk,
        &local.identity_wire(),
        sender_identity_wire,
    )?;
    open_symmetric(&key, &sealed[1..])
}

pub fn call_envelope_to_frame_bytes(env: &CallSigEnvelope) -> Result<Vec<u8>, String> {
    let json = serde_json::to_vec(env).map_err(|e| format!("encode call envelope: {e}"))?;
    let len = u32::try_from(json.len()).map_err(|_| "call envelope too large".to_string())?;
    let mut out = Vec::with_capacity(4 + json.len());
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&json);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::create_keystore_v1;
    use crate::transport_kem_v1::generate_transport_keypair;

    fn transport_pair() -> (StaticSecret, [u8; 32], StaticSecret, [u8; 32]) {
        let (sk_a, pk_a) = generate_transport_keypair();
        let (sk_b, pk_b) = generate_transport_keypair();
        (sk_a, pk_a, sk_b, pk_b)
    }

    #[test]
    fn call_invite_roundtrip() {
        let (_ks_a, alice) = create_keystore_v1("pw", None).unwrap();
        let (_ks_b, bob) = create_keystore_v1("pw2", None).unwrap();
        let (sk_a, pk_a, sk_b, pk_b) = transport_pair();
        let env = build_call_envelope(
            "sig-1",
            "call-abc",
            CallSigKind::Invite,
            &alice,
            &bob.identity_wire(),
            serde_json::json!({ "sdp": "v=0" }),
            1_700_000_000_000,
            CallSealTransportCtx {
                local_sk: &sk_a,
                peer_pk: &pk_b,
            },
        )
        .unwrap();
        let frame = call_envelope_to_frame_bytes(&env).unwrap();
        assert_eq!(frame_wire_share(&frame).unwrap(), CALL_SHARE);
        let env2 = call_envelope_from_frame(&frame).unwrap();
        let parsed = parse_call_envelope_with_transport(
            &env2,
            &bob,
            Some(CallOpenTransportCtx {
                local_sk: &sk_b,
                peer_pk: &pk_a,
            }),
        )
        .unwrap();
        assert_eq!(parsed.call_id, "call-abc");
        assert_eq!(parsed.kind, CallSigKind::Invite);
        assert_eq!(parsed.payload["sdp"], "v=0");
    }

    #[test]
    fn outbound_call_uses_transport_cipher() {
        let (_ks_a, alice) = create_keystore_v1("pw", None).unwrap();
        let (_ks_b, bob) = create_keystore_v1("pw2", None).unwrap();
        let (sk_a, pk_a, sk_b, pk_b) = transport_pair();
        let env = build_call_envelope(
            "sig-2",
            "call-x",
            CallSigKind::Accept,
            &alice,
            &bob.identity_wire(),
            serde_json::json!({ "ok": true }),
            2,
            CallSealTransportCtx {
                local_sk: &sk_a,
                peer_pk: &pk_b,
            },
        )
        .unwrap();
        let sealed = hex::decode(&env.ciphertext_hex).unwrap();
        assert_eq!(sealed.first(), Some(&CALL_CIPHER_TRANSPORT_V2));
        let parsed = parse_call_envelope_with_transport(
            &env,
            &bob,
            Some(CallOpenTransportCtx {
                local_sk: &sk_b,
                peer_pk: &pk_a,
            }),
        )
        .unwrap();
        assert_eq!(parsed.payload["ok"], true);
    }
}
