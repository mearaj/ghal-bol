//! In-memory voice-call state (one active call per remote pubkey).

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use crate::call_sig_v1::CallSigKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallPhase {
    Idle,
    OutgoingRinging,
    IncomingRinging,
    Connected,
}

#[derive(Clone, Debug)]
struct PeerCall {
    call_id: String,
    phase: CallPhase,
    video_enabled: bool,
}

fn store() -> &'static Mutex<HashMap<String, PeerCall>> {
    static S: OnceLock<Mutex<HashMap<String, PeerCall>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn clear_all_calls() {
    if let Ok(mut g) = store().lock() {
        g.clear();
    }
}

pub fn clear_peer(peer_pk_hex: &str) {
    let key = peer_pk_hex.trim().to_ascii_lowercase();
    if let Ok(mut g) = store().lock() {
        g.remove(&key);
    }
}

pub fn peer_call_phase(peer_pk_hex: &str) -> CallPhase {
    let key = peer_pk_hex.trim().to_ascii_lowercase();
    store()
        .lock()
        .ok()
        .and_then(|g| g.get(&key).map(|c| c.phase))
        .unwrap_or(CallPhase::Idle)
}

#[cfg(test)]
pub fn phase_for_peer(peer_pk_hex: &str) -> CallPhase {
    snapshot_for_peer(peer_pk_hex).phase
}

#[cfg(test)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallSnapshot {
    pub phase: CallPhase,
    pub call_id: Option<String>,
    pub video_enabled: bool,
}

#[cfg(test)]
pub fn snapshot_for_peer(peer_pk_hex: &str) -> CallSnapshot {
    let key = peer_pk_hex.trim().to_ascii_lowercase();
    store()
        .lock()
        .ok()
        .and_then(|g| g.get(&key).cloned())
        .map(|c| CallSnapshot {
            phase: c.phase,
            call_id: Some(c.call_id),
            video_enabled: c.video_enabled,
        })
        .unwrap_or(CallSnapshot {
            phase: CallPhase::Idle,
            call_id: None,
            video_enabled: false,
        })
}

/// Apply an **outbound** signal before enqueueing to the network.
pub fn apply_outbound(peer_pk_hex: &str, call_id: &str, kind: CallSigKind) -> Result<(), String> {
    let key = peer_pk_hex.trim().to_ascii_lowercase();
    let mut g = store().lock().map_err(|_| "call state lock".to_string())?;
    let cur = g.get(&key).cloned();
    match kind {
        CallSigKind::Invite => {
            if cur.is_some_and(|c| c.phase != CallPhase::Idle) {
                return Err("call already active with this contact".to_string());
            }
            g.insert(
                key,
                PeerCall {
                    call_id: call_id.to_string(),
                    phase: CallPhase::OutgoingRinging,
                    video_enabled: false,
                },
            );
        }
        CallSigKind::Accept => {
            let Some(c) = cur else {
                return Err("no call to accept".to_string());
            };
            if c.call_id != call_id {
                return Err("call_id mismatch".to_string());
            }
            if c.phase != CallPhase::IncomingRinging {
                return Err("call not in incoming_ringing".to_string());
            }
            g.insert(
                key,
                PeerCall {
                    call_id: call_id.to_string(),
                    phase: CallPhase::Connected,
                    video_enabled: c.video_enabled,
                },
            );
        }
        CallSigKind::Reject | CallSigKind::Hangup => {
            g.remove(&key);
        }
        CallSigKind::VideoOn => {
            let Some(c) = cur else {
                return Err("no active call".to_string());
            };
            if c.call_id != call_id {
                return Err("call_id mismatch".to_string());
            }
            if c.phase != CallPhase::Connected {
                return Err("video_on only when connected".to_string());
            }
            g.insert(
                key,
                PeerCall {
                    call_id: call_id.to_string(),
                    phase: CallPhase::Connected,
                    video_enabled: true,
                },
            );
        }
        CallSigKind::VideoOff => {
            let Some(c) = cur else {
                return Err("no active call".to_string());
            };
            if c.call_id != call_id {
                return Err("call_id mismatch".to_string());
            }
            g.insert(
                key,
                PeerCall {
                    call_id: call_id.to_string(),
                    phase: c.phase,
                    video_enabled: false,
                },
            );
        }
        CallSigKind::SdpOffer | CallSigKind::SdpAnswer | CallSigKind::Ice => {
            let Some(c) = cur else {
                return Err("no active call".to_string());
            };
            if c.call_id != call_id {
                return Err("call_id mismatch".to_string());
            }
        }
    }
    Ok(())
}

/// Apply an **inbound** signal after cryptographic verify.
pub fn apply_inbound(peer_pk_hex: &str, call_id: &str, kind: CallSigKind) -> Result<(), String> {
    let key = peer_pk_hex.trim().to_ascii_lowercase();
    let mut g = store().lock().map_err(|_| "call state lock".to_string())?;
    let cur = g.get(&key).cloned();
    match kind {
        CallSigKind::Invite => {
            match cur {
                None => {
                    g.insert(
                        key,
                        PeerCall {
                            call_id: call_id.to_string(),
                            phase: CallPhase::IncomingRinging,
                            video_enabled: false,
                        },
                    );
                }
                Some(c) if c.phase == CallPhase::OutgoingRinging => {
                    // Glare: both sides tapped Call — lower call_id is the canonical call.
                    if call_id < c.call_id.as_str() {
                        g.insert(
                            key,
                            PeerCall {
                                call_id: call_id.to_string(),
                                phase: CallPhase::IncomingRinging,
                                video_enabled: false,
                            },
                        );
                    } else {
                        return Err("busy".to_string());
                    }
                }
                Some(_) => return Err("busy".to_string()),
            }
        }
        CallSigKind::Accept => {
            let Some(c) = cur else {
                return Err("no outgoing call".to_string());
            };
            if c.call_id != call_id {
                return Err("call_id mismatch".to_string());
            }
            g.insert(
                key,
                PeerCall {
                    call_id: call_id.to_string(),
                    phase: CallPhase::Connected,
                    video_enabled: c.video_enabled,
                },
            );
        }
        CallSigKind::Reject | CallSigKind::Hangup => {
            g.remove(&key);
        }
        CallSigKind::VideoOn => {
            let Some(c) = cur else {
                return Err("no active call".to_string());
            };
            if c.call_id != call_id {
                return Err("call_id mismatch".to_string());
            }
            g.insert(
                key,
                PeerCall {
                    call_id: call_id.to_string(),
                    phase: c.phase,
                    video_enabled: true,
                },
            );
        }
        CallSigKind::VideoOff => {
            let Some(c) = cur else {
                return Err("no active call".to_string());
            };
            if c.call_id != call_id {
                return Err("call_id mismatch".to_string());
            }
            g.insert(
                key,
                PeerCall {
                    call_id: call_id.to_string(),
                    phase: c.phase,
                    video_enabled: false,
                },
            );
        }
        CallSigKind::SdpOffer | CallSigKind::SdpAnswer | CallSigKind::Ice => {
            let Some(c) = cur else {
                return Err("no active call".to_string());
            };
            if c.call_id != call_id {
                return Err("call_id mismatch".to_string());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::call_sig_v1::CallSigKind;

    const PK: &str = "0305b1b0d27745e0a38a7254ea100abc38857b51ded2ac7ea88d3063fb8da21784";

    #[test]
    fn glare_simultaneous_outbound_invites() {
        clear_all_calls();
        apply_outbound(PK, "call-bbb", CallSigKind::Invite).unwrap();
        assert_eq!(phase_for_peer(PK), CallPhase::OutgoingRinging);
        apply_inbound(PK, "call-aaa", CallSigKind::Invite).unwrap();
        let snap = snapshot_for_peer(PK);
        assert_eq!(snap.phase, CallPhase::IncomingRinging);
        assert_eq!(snap.call_id.as_deref(), Some("call-aaa"));

        clear_all_calls();
        apply_outbound(PK, "call-aaa", CallSigKind::Invite).unwrap();
        assert!(apply_inbound(PK, "call-bbb", CallSigKind::Invite).is_err());
        assert_eq!(phase_for_peer(PK), CallPhase::OutgoingRinging);
        assert_eq!(
            snapshot_for_peer(PK).call_id.as_deref(),
            Some("call-aaa")
        );
    }
}
