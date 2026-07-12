//! WAN coordination between peer libp2p, coord HTTP, and the co-located relay (TRANSPORT.md § WAN coordination).
//!
//! **Single owner** for phase transitions and the effects they imply. The libp2p swarm loop
//! reports transport events here; [`WanCoordEffect`] lists what to run next — no scattered
//! coord/relay policy in `chat_server.rs`.
//!
//! Invariants:
//! - **Parallel LAN + WAN:** mDNS/direct and coord relay circuit run together per node and per peer.
//!   Neither path is torn down because the other succeeded; both active simultaneously is correct.
//!   One DM stream mux; stream may prefer direct when both links exist — never `disconnect_peer` for WAN fix.
//! - Our relay's `/p2p-circuit` is registered on coord **only** by the relay server on reservation.
//! - `coord_registered` (client) is set only after HTTP self-lookup confirms a WAN-dialable row **and**
//!   local relay circuit listen is up (CGNAT path), or after client register + self-lookup (public TCP).
//! - Remote circuit dials that fail with `NoReservation` treat coord presence as stale (re-lookup).

use std::sync::atomic::{AtomicBool, Ordering};

static LOCAL_RELAY_LISTENING: AtomicBool = AtomicBool::new(false);

/// Snapshot from the libp2p swarm loop when relay circuit listen addr is up or down.
pub fn sync_local_relay_circuit_listening(circuit_listening: bool) {
    LOCAL_RELAY_LISTENING.store(circuit_listening, Ordering::Relaxed);
}

pub fn local_relay_circuit_listening() -> bool {
    LOCAL_RELAY_LISTENING.load(Ordering::Relaxed)
}

/// Side effects for the swarm loop after a WAN coordination event (TRANSPORT.md § event-driven sync).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WanCoordEffect {
    /// `coord_runtime::mark_coord_relay_hop_lost()` — bootstrap HOP dropped; keep registered if circuit listen up.
    MarkRelayHopLost,
    /// Relay reservation accepted or circuit listen up — poll coord self until mirror visible.
    ScheduleCoordPresenceAfterRelay,
    /// Relay circuit listener closed — re-poll + re-register path.
    ScheduleCoordPresencePoll,
    /// `notify_relay_refresh()` — deferred reserve pass on next upkeep tick.
    NotifyRelayRefresh,
    /// `notify_coord_lookup()` — urgent remote peer re-lookup.
    NotifyCoordLookup,
    /// `notify_dm_presence_wake()` — wake rediscovery for known DM peers.
    NotifyDmPresenceWake,
    /// Remote dial failed — clear lookup backoff for that pk.
    MarkRemoteCircuitStale(String),
    /// `coord_runtime::coord_note_relay_reservation` — preferred relay peer id.
    NoteRelayReservation,
    /// Reopen DM chat streams on existing libp2p links (mux failover direct → relay).
    NotifyStreamReopen,
    /// Mark all DM contacts urgent for phase E–F (coord lookup + circuit dial).
    MarkAllDmReconnectUrgent,
}

/// Local device left Wi‑Fi/LAN (e.g. mobile-data). TRANSPORT.md § “LAN ↔ WAN handover”.
///
/// Parallel LAN+WAN: purge LAN discovery state, keep relay links up, drive phases B–D + E–F.
/// Does **not** disconnect relay circuits — WAN must already be warm.
pub fn on_left_lan() -> Vec<WanCoordEffect> {
    vec![
        WanCoordEffect::NotifyRelayRefresh,
        WanCoordEffect::ScheduleCoordPresencePoll,
        WanCoordEffect::ScheduleCoordPresenceAfterRelay,
        WanCoordEffect::NotifyCoordLookup,
        WanCoordEffect::NotifyDmPresenceWake,
        WanCoordEffect::NotifyStreamReopen,
        WanCoordEffect::MarkAllDmReconnectUrgent,
    ]
}

/// Remote contact no longer on local LAN (mDNS expired / we left LAN).
pub fn on_peer_off_local_lan(public_key_hex: &str) -> Vec<WanCoordEffect> {
    vec![
        WanCoordEffect::MarkRemoteCircuitStale(public_key_hex.to_string()),
        WanCoordEffect::NotifyCoordLookup,
        WanCoordEffect::NotifyStreamReopen,
    ]
}

/// Local device returned to Wi‑Fi/LAN — parallel WAN kept; rediscover contacts on LAN.
pub fn on_lan_path_restored() -> Vec<WanCoordEffect> {
    vec![
        WanCoordEffect::NotifyRelayRefresh,
        WanCoordEffect::ScheduleCoordPresenceAfterRelay,
        WanCoordEffect::NotifyDmPresenceWake,
        WanCoordEffect::NotifyStreamReopen,
        WanCoordEffect::MarkAllDmReconnectUrgent,
    ]
}

/// Coord-relay bootstrap TCP lost (parallel LAN+WAN — do not purge coord on client disconnect).
pub fn on_relay_bootstrap_lost(circuit_still_listening: bool) -> Vec<WanCoordEffect> {
    let mut out = vec![
        WanCoordEffect::MarkRelayHopLost,
        WanCoordEffect::ScheduleCoordPresencePoll,
        WanCoordEffect::NotifyRelayRefresh,
        WanCoordEffect::NotifyCoordLookup,
    ];
    if circuit_still_listening {
        out.push(WanCoordEffect::ScheduleCoordPresenceAfterRelay);
    }
    out
}

/// Relay `ReservationReqAccepted` — server upserts circuit; client polls self-lookup (phase D).
pub fn on_reservation_accepted() -> Vec<WanCoordEffect> {
    vec![
        WanCoordEffect::NoteRelayReservation,
        WanCoordEffect::ScheduleCoordPresenceAfterRelay,
        WanCoordEffect::NotifyDmPresenceWake,
    ]
}

/// Relay circuit `/p2p-circuit` listener is up (NewListenAddr).
pub fn on_relay_circuit_listening() -> Vec<WanCoordEffect> {
    vec![WanCoordEffect::ScheduleCoordPresenceAfterRelay]
}

/// IPv4 relay circuit listener closed and not immediately renewed.
pub fn on_relay_circuit_lost() -> Vec<WanCoordEffect> {
    vec![
        WanCoordEffect::NotifyRelayRefresh,
        WanCoordEffect::ScheduleCoordPresenceAfterRelay,
        WanCoordEffect::NotifyCoordLookup,
        WanCoordEffect::NotifyDmPresenceWake,
    ]
}

/// Outbound circuit dial to a remote peer failed — coord row may be stale vs relay live gate.
pub fn on_remote_circuit_dial_failed(public_key_hex: &str, err: &str) -> Vec<WanCoordEffect> {
    let stale = err.contains("NoReservation")
        || err.contains("Relay has no reservation")
        || err.contains("Timeout has been reached")
        || err.contains("ConnectionFailed")
        || err.contains("unexpected end of file")
        || err.contains("Handshake failed");
    if stale {
        vec![
            WanCoordEffect::MarkRemoteCircuitStale(public_key_hex.to_string()),
            WanCoordEffect::NotifyCoordLookup,
        ]
    } else {
        vec![]
    }
}

/// Human-readable phase for `Native/flow` connectivity lines.
pub fn phase_label(
    coord_configured: bool,
    coord_registered: bool,
    coord_http_degraded: bool,
    bootstrap_ok: bool,
    circuit_listening: bool,
) -> &'static str {
    if !coord_configured {
        return "unconfigured";
    }
    if circuit_listening && coord_registered && !coord_http_degraded {
        return "wan_ready";
    }
    if coord_registered && coord_http_degraded {
        return "http_degraded";
    }
    if circuit_listening && !coord_registered {
        return "awaiting_coord_mirror";
    }
    if bootstrap_ok && !circuit_listening {
        return "awaiting_relay_circuit";
    }
    if coord_configured {
        "awaiting_relay_bootstrap"
    } else {
        "unconfigured"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_wan_ready_requires_circuit_and_coord() {
        assert_eq!(phase_label(true, true, false, true, true), "wan_ready");
        assert_ne!(phase_label(true, true, false, true, false), "wan_ready");
    }

    #[test]
    fn coord_mirror_requires_local_listen() {
        sync_local_relay_circuit_listening(false);
        assert!(!local_relay_circuit_listening());
        sync_local_relay_circuit_listening(true);
        assert!(local_relay_circuit_listening());
    }

    #[test]
    fn registered_without_circuit_is_not_wan_ready() {
        assert_eq!(
            phase_label(true, true, false, true, false),
            "awaiting_relay_circuit"
        );
    }

    #[test]
    fn bootstrap_lost_keeps_presence_poll_when_circuit_up() {
        let effects = on_relay_bootstrap_lost(true);
        assert!(effects.contains(&WanCoordEffect::MarkRelayHopLost));
        assert!(effects.contains(&WanCoordEffect::ScheduleCoordPresenceAfterRelay));
    }

    #[test]
    fn remote_no_reservation_triggers_stale_and_lookup() {
        let effects = on_remote_circuit_dial_failed(
            "aa".repeat(32).as_str(),
            "Relay has no reservation for destination",
        );
        assert_eq!(effects.len(), 2);
        assert!(matches!(
            effects[0],
            WanCoordEffect::MarkRemoteCircuitStale(_)
        ));
        assert!(effects.contains(&WanCoordEffect::NotifyCoordLookup));
    }

    #[test]
    fn left_lan_drives_wan_phases_without_disconnect() {
        let effects = on_left_lan();
        assert!(effects.contains(&WanCoordEffect::NotifyRelayRefresh));
        assert!(effects.contains(&WanCoordEffect::NotifyCoordLookup));
        assert!(effects.contains(&WanCoordEffect::NotifyStreamReopen));
        assert!(effects.contains(&WanCoordEffect::MarkAllDmReconnectUrgent));
        assert!(effects.contains(&WanCoordEffect::ScheduleCoordPresenceAfterRelay));
    }

    #[test]
    fn lan_path_restored_wakes_lan_rediscovery() {
        let effects = on_lan_path_restored();
        assert!(effects.contains(&WanCoordEffect::NotifyRelayRefresh));
        assert!(effects.contains(&WanCoordEffect::ScheduleCoordPresenceAfterRelay));
        assert!(effects.contains(&WanCoordEffect::NotifyDmPresenceWake));
        assert!(effects.contains(&WanCoordEffect::NotifyStreamReopen));
        assert!(effects.contains(&WanCoordEffect::MarkAllDmReconnectUrgent));
    }

    #[test]
    fn handshake_eof_triggers_stale_lookup() {
        let pk = "02".repeat(33);
        let effects = on_remote_circuit_dial_failed(&pk, "Handshake failed: unexpected end of file");
        assert_eq!(effects.len(), 2);
        assert!(matches!(
            effects[0],
            WanCoordEffect::MarkRemoteCircuitStale(_)
        ));
    }
}
