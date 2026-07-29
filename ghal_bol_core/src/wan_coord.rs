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

pub fn local_relay_circuit_listening() -> bool {
    LOCAL_RELAY_LISTENING.load(Ordering::Relaxed)
}
