//! In-memory voice-call state (one active call per remote pubkey).

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::call_sig_v1::CallSigKind;

/// Match Flutter `CallController._maxLiveInviteAgeMs` — invites and ringing UI must not outlive this.
pub const MAX_LIVE_CALL_INVITE_AGE_MS: i64 = 45_000;

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn call_invite_is_live(created_at_ms: i64, now_ms: i64) -> bool {
    if created_at_ms <= 0 {
        return false;
    }
    let age = now_ms.saturating_sub(created_at_ms);
    age >= 0 && age <= MAX_LIVE_CALL_INVITE_AGE_MS
}

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
    /// When [phase] last changed — used to expire stuck ringing after hangup loss.
    phase_at_ms: i64,
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

/// Active `(peer_pk_hex, call_id)` pairs for privacy teardown when the UI session ends.
pub fn store_for_teardown() -> Result<Vec<(String, String)>, ()> {
    let Ok(g) = store().lock() else {
        return Err(());
    };
    Ok(g.iter()
        .map(|(pk, c)| (pk.clone(), c.call_id.clone()))
        .collect())
}

/// Drop [CallPhase::IncomingRinging] / [CallPhase::OutgoingRinging] older than [MAX_LIVE_CALL_INVITE_AGE_MS].
pub fn expire_stale_ringing(now_ms: i64) -> bool {
    let Ok(mut g) = store().lock() else {
        return false;
    };
    let before = g.len();
    g.retain(|_, c| {
        !matches!(
            c.phase,
            CallPhase::IncomingRinging | CallPhase::OutgoingRinging
        ) || now_ms.saturating_sub(c.phase_at_ms) <= MAX_LIVE_CALL_INVITE_AGE_MS
    });
    g.len() != before
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

/// First inbound ring waiting for the user (for UI restore when the app was killed).
pub fn first_incoming_ringing() -> Option<(String, String)> {
    expire_stale_ringing(now_ms());
    let Ok(g) = store().lock() else {
        return None;
    };
    for (pk, call) in g.iter() {
        if call.phase == CallPhase::IncomingRinging {
            return Some((pk.clone(), call.call_id.clone()));
        }
    }
    None
}

/// Age of the current inbound ring, for UI stale checks.
pub fn incoming_ring_age_ms(now_ms: i64) -> Option<i64> {
    expire_stale_ringing(now_ms);
    let Ok(g) = store().lock() else {
        return None;
    };
    for call in g.values() {
        if call.phase == CallPhase::IncomingRinging {
            return Some(now_ms.saturating_sub(call.phase_at_ms));
        }
    }
    None
}

/// True while our outbound `invite` for [call_id] is still the active ringing call.
pub fn outbound_invite_active(peer_pk_hex: &str, call_id: &str) -> bool {
    let key = peer_pk_hex.trim().to_ascii_lowercase();
    let cid = call_id.trim();
    store()
        .lock()
        .ok()
        .and_then(|g| {
            g.get(&key)
                .map(|c| c.phase == CallPhase::OutgoingRinging && c.call_id == cid)
        })
        .unwrap_or(false)
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

fn fresh_call(call_id: String, phase: CallPhase, video_enabled: bool) -> PeerCall {
    PeerCall {
        call_id,
        phase,
        video_enabled,
        phase_at_ms: now_ms(),
    }
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
                fresh_call(call_id.to_string(), CallPhase::OutgoingRinging, false),
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
                fresh_call(call_id.to_string(), CallPhase::Connected, c.video_enabled),
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
                fresh_call(call_id.to_string(), CallPhase::Connected, true),
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
                    phase_at_ms: c.phase_at_ms,
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
                        fresh_call(call_id.to_string(), CallPhase::IncomingRinging, false),
                    );
                }
                Some(c) if c.phase == CallPhase::OutgoingRinging => {
                    // Glare: both sides tapped Call — lower call_id is the canonical call.
                    if call_id < c.call_id.as_str() {
                        g.insert(
                            key,
                            fresh_call(call_id.to_string(), CallPhase::IncomingRinging, false),
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
                fresh_call(call_id.to_string(), CallPhase::Connected, c.video_enabled),
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
                    phase_at_ms: c.phase_at_ms,
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
                    phase_at_ms: c.phase_at_ms,
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
    use std::sync::{Mutex, OnceLock};

    fn lock_call_state_tests() -> std::sync::MutexGuard<'static, ()> {
        static L: OnceLock<Mutex<()>> = OnceLock::new();
        L.get_or_init(|| Mutex::new(())).lock().unwrap()
    }

    #[test]
    fn glare_simultaneous_outbound_invites() {
        let _lock = lock_call_state_tests();
        // Each test must use a unique key because `call_state` is global process state and
        // lib tests run concurrently.
        const PK: &str = "0305b1b0d27745e0a38a7254ea100abc38857b51ded2ac7ea88d3063fb8da21784";
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
        assert_eq!(snapshot_for_peer(PK).call_id.as_deref(), Some("call-aaa"));
    }

    #[test]
    fn expire_stale_ringing_clears_old_inbound() {
        let _lock = lock_call_state_tests();
        const PK: &str = "03f1b1b0d27745e0a38a7254ea100abc38857b51ded2ac7ea88d3063fb8da21784";
        clear_all_calls();
        let t0 = now_ms();
        apply_inbound(PK, "call-stale", CallSigKind::Invite).unwrap();
        assert_eq!(phase_for_peer(PK), CallPhase::IncomingRinging);
        assert!(expire_stale_ringing(
            t0 + MAX_LIVE_CALL_INVITE_AGE_MS + 1_000
        ));
        assert_eq!(phase_for_peer(PK), CallPhase::Idle);
        assert!(first_incoming_ringing().is_none());
    }
}
