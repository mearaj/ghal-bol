//! Voice-call **signaling** envelopes on the DM stream (`ghal_bol_call_v1`).
//!
//! Media (WebRTC / Opus) is phase 2; this module is sign, seal, parse only.

use libp2p_identity::Keypair;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use secp256k1::SecretKey;

use crate::public_key_util::secp256k1_public_key_from_hex;
use crate::secp256k1_seal::{open_sealed_secp256k1, seal_to_secp256k1_public};

pub const CALL_SHARE: &str = "ghal_bol_call_v1";
pub const CALL_FORMAT_VERSION: u64 = 1;

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
    /// In-call request to enable video (followed by SDP renegotiation).
    VideoOn,
    /// Disable video; audio continues.
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

/// Same JSON shape as [`MsgEnvelope`] but different `ghalbol.share` / `kind` enum.
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

fn envelope_recipient_ok(env: &CallSigEnvelope, my_public_key_hex: &str) -> bool {
    env.recipient_public_key_hex.trim() == my_public_key_hex.trim()
}

fn canonical_sign_bytes(env: &CallSigEnvelope) -> Result<Vec<u8>, String> {
    let mut clone = env.clone();
    clone.signature_hex = None;
    serde_json::to_vec(&clone).map_err(|e| format!("canonical json: {e}"))
}

pub fn sign_call_envelope(env: &mut CallSigEnvelope, sender: &Keypair) -> Result<(), String> {
    let bytes = canonical_sign_bytes(env)?;
    let sig = sender
        .sign(&bytes)
        .map_err(|e| format!("sign: {e}"))?;
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
    let sender_pk = secp256k1_public_key_from_hex(env.sender_public_key_hex.trim())?;
    let libp2p_pk = libp2p_identity::PublicKey::from(sender_pk);
    let bytes = canonical_sign_bytes(env)?;
    if !libp2p_pk.verify(&bytes, &sig) {
        return Err("signature verification failed".to_string());
    }
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
    sender: &Keypair,
    recipient_public_key_hex: &str,
    payload: Value,
    created_at_ms: i64,
) -> Result<CallSigEnvelope, String> {
    let sender_pk = sender
        .public()
        .try_into_secp256k1()
        .map_err(|e| format!("sender key: {e}"))?;
    let sender_hex = crate::public_key_util::secp256k1_public_key_to_hex(&sender_pk);
    let recipient = recipient_public_key_hex.trim();
    let recipient_pk = secp256k1_public_key_from_hex(recipient)?;
    let inner_bytes = serde_json::to_vec(&payload).unwrap_or_else(|_| b"{}".to_vec());
    let sealed = if inner_bytes == b"{}" {
        seal_to_secp256k1_public(&recipient_pk.to_bytes(), b"{}")?
    } else {
        seal_to_secp256k1_public(&recipient_pk.to_bytes(), &inner_bytes)?
    };
    let mut env = CallSigEnvelope {
        wire_share: CALL_SHARE.to_string(),
        format_version: CALL_FORMAT_VERSION,
        id: id.to_string(),
        kind,
        ref_id: Some(call_id.to_string()),
        sender_public_key_hex: sender_hex,
        recipient_public_key_hex: recipient.to_string(),
        created_at_ms,
        ciphertext_hex: hex::encode(sealed),
        signature_hex: None,
    };
    sign_call_envelope(&mut env, sender)?;
    Ok(env)
}

pub fn parse_call_envelope(
    env: &CallSigEnvelope,
    my_public_key_hex: &str,
    my_secret: &SecretKey,
) -> Result<ParsedCallSignal, String> {
    verify_call_envelope(env)?;
    if !envelope_recipient_ok(env, my_public_key_hex) {
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
        let sealed = hex::decode(env.ciphertext_hex.trim())
            .map_err(|e| format!("ciphertext hex: {e}"))?;
        let plain = open_sealed_secp256k1(my_secret, &sealed)?;
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

    #[test]
    fn call_invite_roundtrip() {
        let (_ks_a, alice) = create_keystore_v1("pw", None).unwrap();
        let (_ks_b, bob) = create_keystore_v1("pw2", None).unwrap();
        let env = build_call_envelope(
            "sig-1",
            "call-abc",
            CallSigKind::Invite,
            alice.keypair(),
            &bob.public_key_hex(),
            serde_json::json!({ "sdp": "v=0" }),
            1_700_000_000_000,
        )
        .unwrap();
        let frame = call_envelope_to_frame_bytes(&env).unwrap();
        assert_eq!(frame_wire_share(&frame).unwrap(), CALL_SHARE);
        let env2 = call_envelope_from_frame(&frame).unwrap();
        let parsed =
            parse_call_envelope(&env2, &bob.public_key_hex(), bob.secp256k1_secret()).unwrap();
        assert_eq!(parsed.call_id, "call-abc");
        assert_eq!(parsed.kind, CallSigKind::Invite);
        assert_eq!(parsed.payload["sdp"], "v=0");
    }
}
