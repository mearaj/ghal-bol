const COORD_LOOKUP_INTERVAL_SECS: u64 = 2;
/// Min gap between coord HTTP lookups for a disconnected DM peer (dm_upkeep ~1s tick).
const DM_COORD_LOOKUP_MIN_INTERVAL_MS: i64 = 2_000;
/// Scale guardrail (TRANSPORT.md § "Instant connect at any roster size"). Peers with **active
/// intent** (pending outbox or the foreground chat) are looked up every tick uncapped, so a
/// reachable peer always connects within seconds. The remaining **idle** contacts — possibly
/// thousands of stale/offline/404 entries — are swept LRU (oldest-looked-up first), at most this
/// many coord HTTP lookups per upkeep tick. This bounds per-tick HTTP and, crucially, keeps the
/// sequential `await` chain short so stale peers can never delay a peer that wants to connect now.
const COORD_BACKGROUND_LOOKUPS_PER_TICK: usize = 24;
const NETWORK_PROFILE_POLL_SECS: u64 = 1;
const PEER_LAN_SEEN_TTL_MS: i64 = 180_000;
const BOOTSTRAP_REDIAL_INTERVAL_SECS: u64 = 12;
/// After a DM connection drops, treat reconnect as urgent for this long: coord lookups skip the
/// `peer_not_on_server` backoff and run every ~1s upkeep tick. Bounded so a genuinely offline
/// peer eventually falls back to the normal coord cadence + exponential backoff.
const DM_RECONNECT_URGENT_WINDOW_MS: i64 = 30_000;

/// A ready coord lookup result becomes stale (re-fetched) if the swarm loop has not consumed it
/// within this window — coord presence (relay port) is live data, never trusted when old.
const COORD_LOOKUP_RESULT_STALE_MS: i64 = 15_000;

/// Outcome of a single background coord HTTP lookup, applied on the swarm loop next tick.
enum CoordLookupOutcome {
    Ok(Vec<Multiaddr>),
    Err(String),
}

struct CoordLookupResult {
    outcome: CoordLookupOutcome,
    fetched_ms: i64,
}

/// Ready (unconsumed) background lookup results, keyed by peer public key hex.
fn coord_lookup_results() -> &'static Mutex<HashMap<String, CoordLookupResult>> {
    static R: OnceLock<Mutex<HashMap<String, CoordLookupResult>>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Public keys with a coord HTTP lookup currently running, so we never spawn a duplicate.
fn coord_lookup_in_flight() -> &'static Mutex<HashSet<String>> {
    static F: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    F.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Spawn a **background** coord HTTP lookup for `pk` (fire-and-forget). The swarm event loop must
/// never `.await` coord HTTP: doing so freezes libp2p for the whole request, so the inbound relay
/// `STOP` substream times out and circuit forwarding fails (root cause of WAN-only chat failure —
/// AGENTS.md golden rule 9 / prime directive). Instead we kick the lookup here and consume the
/// result synchronously on the next pass via [`drain_ready_coord_lookups`]. Deduped per pk while a
/// request is in flight; on completion the result is stored and the swarm loop is woken via
/// `notify_coord_lookup()`.
fn request_coord_lookup(pk: &str) {
    let pk = pk.trim().to_string();
    if pk.len() != 66 {
        return;
    }
    {
        let Ok(mut inflight) = coord_lookup_in_flight().lock() else {
            return;
        };
        if !inflight.insert(pk.clone()) {
            return; // already running for this pk
        }
    }
    tokio::spawn(async move {
        let outcome =
            match crate::coord_runtime::lookup_dial_multiaddrs_for_public_key_async(&pk).await {
                Ok(addrs) => CoordLookupOutcome::Ok(addrs),
                Err(e) => CoordLookupOutcome::Err(e),
            };
        if let Ok(mut r) = coord_lookup_results().lock() {
            r.insert(
                pk.clone(),
                CoordLookupResult {
                    outcome,
                    fetched_ms: chrono_now_ms(),
                },
            );
        }
        if let Ok(mut f) = coord_lookup_in_flight().lock() {
            f.remove(&pk);
        }
        // Wake the swarm loop so it applies the result (dials) without waiting for the next tick.
        notify_coord_lookup();
    });
}

/// Drain all ready (fresh) background lookup results so the swarm loop can apply them (dial) this
/// pass. Stale results are dropped — relay presence is live data and a stale port could break WAN.
fn drain_ready_coord_lookups(now_ms: i64) -> Vec<(String, CoordLookupOutcome)> {
    let mut out = Vec::new();
    if let Ok(mut r) = coord_lookup_results().lock() {
        let stale_cutoff = now_ms.saturating_sub(COORD_LOOKUP_RESULT_STALE_MS);
        let pks: Vec<String> = r.keys().cloned().collect();
        for pk in pks {
            if let Some(res) = r.remove(&pk) {
                if res.fetched_ms < stale_cutoff {
                    continue; // too old — let the next pass re-request a live lookup
                }
                out.push((pk, res.outcome));
            }
        }
    }
    out
}

/// Live LAN reachability — mDNS candidate only (not `peers_on_local_lan` TTL stamps).
fn peer_has_live_mdns_lan(session: &SessionState, peer: PeerId) -> bool {
    session.peer_mdns_lan_addr(peer).is_some()
}

fn peer_has_lingering_direct(session: &SessionState, peer: PeerId) -> bool {
    session.peer_has_direct_connection(peer)
        || session
            .dm_direct_conn_ids
            .read()
            .ok()
            .is_some_and(|m| m.get(&peer).is_some_and(|s| !s.is_empty()))
}

/// Sustained-stuck threshold before a lingering direct LAN mux is judged dead and torn down for
/// relay. Must exceed normal ack RTT on a healthy LAN: a working direct link drains text/delivery
/// acks in well under a second, so a frame that is merely *in flight* must never trip this. Only a
/// peer that has truly left Wi‑Fi (writer bound to a dead mux) leaves work unacked this long.
///
/// Regression guard (flutter_linux.log 2026-06-28): the old test was "any pending outbound blocker",
/// so normal back-and-forth chat looked "stuck" on every ~1s upkeep tick → 76× `close stale direct`
/// tore down a *working* LAN mux mid-chat and forced the timing-out relay → multi-second stalls then
/// burst drain ("bulk then stop then bulk"). Time-gating keeps a live LAN link and still recovers a
/// genuinely dead one (TRANSPORT.md § Asymmetric LAN↔WAN mux recovery).
const LAN_HANDOVER_STUCK_MS: i64 = 4_000;

/// LAN soft/full kick is active and outbound (text/delivery acks) has been stuck long enough to
/// prove the lingering direct mux is dead — peer has left LAN for WAN. Transient in-flight frames
/// during healthy chat do **not** qualify (see `LAN_HANDOVER_STUCK_MS`).
fn peer_lan_handover_outbound_stuck(session: &SessionState, peer: PeerId) -> bool {
    session.lan_listen_rediscovery_requested(peer)
        && session.peer_outbound_stuck_for(peer, chrono_now_ms(), LAN_HANDOVER_STUCK_MS)
}

/// Zombie chat mux during documented LAN handover — requires `lan_listen_rediscovery_requested`.
fn peer_needs_wan_mux_reopen(session: &SessionState, peer: PeerId) -> bool {
    peer_lan_handover_outbound_stuck(session, peer) && session.dm_peer_stream_up(peer)
}

/// Stuck threshold before treating an open writer as a zombie mux.
///
/// Relay-only WAN without stale direct: use a longer window — bursty relay delivery routinely
/// exceeds 4s without meaning the mux is dead (TRANSPORT.md § Known symptom — bursty delivery).
/// Asymmetric LAN↔WAN / stale direct: keep the short window.
fn zombie_mux_stuck_threshold_ms(session: &SessionState, peer: PeerId) -> i64 {
    if peer_has_stale_direct_lan_conn(session, peer)
        || peer_wan_asymmetric_mux_likely(session, peer)
        || session.lan_listen_rediscovery_requested(peer)
    {
        LAN_HANDOVER_STUCK_MS
    } else if session.peer_has_relay_connection(peer) {
        12_000
    } else {
        LAN_HANDOVER_STUCK_MS
    }
}

/// Writer looks open (`stream=true`) but outbound has been on the wire long enough without
/// delivery ack — remote may have dropped all paths while libp2p still reports connected
/// (half-open TCP) or we are writing into a dead mux. Does **not** close direct links
/// (TRANSPORT.md § Post-mortem 2026-06-24 class D, § Asymmetric mux recovery).
fn peer_needs_zombie_mux_reopen(session: &SessionState, peer: PeerId) -> bool {
    let now_ms = chrono_now_ms();
    session.dm_peer_stream_up(peer)
        && session.peer_outbound_stuck_for(
            peer,
            now_ms,
            zombie_mux_stuck_threshold_ms(session, peer),
        )
}

/// Wi‑Fi side asymmetric LAN↔WAN — stale mDNS/TTL + stuck outbound while remote peer is on cell.
fn peer_wan_asymmetric_mux_likely(session: &SessionState, peer: PeerId) -> bool {
    peer_lan_handover_outbound_stuck(session, peer)
        && (peer_has_lingering_direct(session, peer) || session.dm_peer_stream_up(peer))
}

/// Parallel LAN+WAN: stream is up on a live path but relay hop is missing — pursue throttled
/// additive relay dial without treating the chat mux as down (TRANSPORT.md § Both links active).
fn needs_additive_relay_dial(session: &SessionState, peer: PeerId, connected: bool) -> bool {
    connected
        && crate::coord_runtime::coord_is_configured()
        && !session.peer_has_relay_connection(peer)
}

/// libp2p-connected on an existing relay hop — one circuit per peer; stream reopen, not re-dial.
fn peer_wan_relay_connected(
    swarm: &Swarm<ChatBehaviour>,
    session: &SessionState,
    peer: PeerId,
) -> bool {
    swarm.is_connected(&peer) && session.peer_has_relay_connection(peer)
}

/// Remote peer is off LAN but we need a WAN relay path (asymmetric handover).
///
/// **Ordering matters (TRANSPORT.md § Parallel LAN + WAN, § Asymmetric mux recovery):**
/// a live mDNS candidate means the peer is *currently on our LAN*, so the direct mux is real and
/// LAN+WAN running in parallel is intentional — never reconcile it as "stale". Only when there is
/// **no** live mDNS do we treat a lingering direct link alongside relay as the asymmetric case
/// (peer moved to mobile-data while we still hold a dead LAN mux).
fn dm_peer_needs_wan_relay_path(session: &SessionState, peer: PeerId) -> bool {
    if !crate::coord_runtime::coord_is_configured() {
        return false;
    }
    if peer_wan_asymmetric_mux_likely(session, peer) {
        return true;
    }
    let has_relay = session.peer_has_relay_connection(peer);
    let has_stale_direct = peer_has_stale_direct_lan_conn(session, peer);
    if has_relay && has_stale_direct {
        return true;
    }
    // Peer is reachable on our LAN right now → parallel LAN+WAN is intended; not a stale mux.
    if peer_has_live_mdns_lan(session, peer) {
        return false;
    }
    !has_relay
}

fn peer_has_stale_direct_lan_conn(session: &SessionState, peer: PeerId) -> bool {
    if !peer_has_lingering_direct(session, peer) {
        return false;
    }
    // mDNS cache or on-LAN TTL can lie after remote Wi‑Fi→mobile handover — trust stuck outbound
    // only during an active LAN handover window (not mere 4s WAN relay ack delay).
    if peer_has_live_mdns_lan(session, peer) {
        return peer_lan_handover_outbound_stuck(session, peer);
    }
    // Fresh private-IP direct connect — parallel relay+direct on Wi‑Fi until outbound proves otherwise.
    if session.peer_on_local_lan(peer) {
        return peer_lan_handover_outbound_stuck(session, peer);
    }
    true
}

/// Wi‑Fi side: mobile peer re-dialed inbound on relay while we still hold direct — set session
/// flag so step 6 adopt + step 7 no-defer fire before stale mDNS / Symptom C (05:56:22 soak).
pub(crate) fn should_mark_relay_inbound_handover(session: &SessionState, peer: PeerId) -> bool {
    if !session.network_profile_snapshot().has_active_lan() {
        return false;
    }
    if session.lan_listen_rediscovery_requested(peer) {
        return true;
    }
    if peer_wan_asymmetric_mux_likely(session, peer) {
        return true;
    }
    if !peer_has_lingering_direct(session, peer) {
        return false;
    }
    let now_ms = chrono_now_ms();
    // In-flight or queued outbound while remote opens relay inbound — not healthy parallel LAN+WAN
    // (direct acks land in well under 1s when the mux is live).
    session.peer_outbound_stuck_for(peer, now_ms, 0) || session.peer_has_pending_outbox(peer)
}

/// Wi‑Fi side relay-only handover: mobile peer re-dialed on relay while we hunt LAN but hold **no**
/// direct `ConnectionId`s — writer may sit on a pre-handover relay mux (flutter_linux.log
/// 2026-07-02 05:29:54). Requires `lan_listen_rediscovery_requested` so healthy parallel
/// LAN+WAN on the same subnet (relay **and** direct both up) stays on `peer_has_stale_direct_lan_conn`.
fn peer_relay_inbound_handover_mux_recovery(session: &SessionState, peer: PeerId) -> bool {
    if !session.network_profile_snapshot().has_active_lan() {
        return false;
    }
    // Event-driven handover — adopt immediately; do not wait for 4s Symptom C or lan rediscovery.
    if session.relay_inbound_handover_active(peer) {
        return true;
    }
    if !session.lan_listen_rediscovery_requested(peer) {
        return false;
    }
    if !session.peer_has_relay_connection(peer) {
        return false;
    }
    if peer_has_lingering_direct(session, peer) {
        return false;
    }
    let now_ms = chrono_now_ms();
    // Symptom C — direct LAN mux still draining; not relay-only WAN handover.
    if session.peer_has_direct_connection(peer)
        && session.dm_peer_stream_up(peer)
        && session.dm_mux_recently_active(peer, now_ms)
        && !session.peer_outbound_stuck_for(peer, now_ms, LAN_HANDOVER_STUCK_MS)
    {
        return false;
    }
    true
}

/// Relay already established but a stale direct LAN mux lingers — recover on the existing relay
/// (close direct + stream reopen), not by opening another circuit (TRANSPORT.md § Asymmetric mux).
/// Also true for relay-only inbound handover (no lingering direct) during `lan_listen_rediscovery`.
pub(crate) fn asymmetric_relay_recover_on_existing_link(session: &SessionState, peer: PeerId) -> bool {
    if peer_wan_asymmetric_mux_likely(session, peer) {
        return true;
    }
    if peer_relay_inbound_handover_mux_recovery(session, peer) {
        return true;
    }
    session.peer_has_relay_connection(peer) && peer_has_stale_direct_lan_conn(session, peer)
}

/// Wi‑Fi side during asymmetric LAN↔WAN: relay is up but we still hold a stale direct mux — defer
/// our outbound `open_stream` and accept the mobile peer's inbound stream instead (avoids symmetric
/// open_stream deadlock on relay). Mobile-data re-dialer must not defer (no active LAN).
fn should_defer_outbound_stream_for_asymmetric_relay(session: &SessionState, peer: PeerId) -> bool {
    if !session.network_profile_snapshot().has_active_lan() {
        return false;
    }
    // Peer already opened inbound on relay — adopt (step 6), do not defer our outbound.
    if session.relay_inbound_handover_active(peer) {
        return false;
    }
    session.peer_has_relay_connection(peer) && peer_has_stale_direct_lan_conn(session, peer)
}

/// Do not open a chat stream on a dead direct LAN mux while WAN relay recovery is pending.
fn should_defer_stream_open_for_wan_mux(session: &SessionState, peer: PeerId) -> bool {
    if !dm_peer_needs_wan_relay_path(session, peer) {
        return false;
    }
    // Relay link already up — open/attach stream on it; reconcile closes stale direct in parallel.
    // Deferring here while relay+stale-direct both exist deadlocked outbox after LAN→WAN (2026-06-25).
    if session.peer_has_relay_connection(peer) {
        return false;
    }
    // No relay yet — do not attach stream to a stale direct mux while coord dials WAN.
    true
}

/// Connected DM with an open chat stream — discovery/coord/identify noop (protonet-as-reference).
fn dm_peer_chat_link_stable(
    swarm: &Swarm<ChatBehaviour>,
    session: &SessionState,
    peer: PeerId,
    pk_hex: Option<&str>,
    now_ms: i64,
) -> bool {
    if session.dm_peer_stream_up(peer) {
        if peer_needs_zombie_mux_reopen(session, peer) {
            return false;
        }
        if session.peer_has_relay_connection(peer) {
            if peer_wan_asymmetric_mux_likely(session, peer) || peer_needs_wan_mux_reopen(session, peer) {
                return false;
            }
            return true;
        }
        if dm_peer_needs_wan_relay_path(session, peer) {
            return false;
        }
        return true;
    }
    if !swarm.is_connected(&peer) {
        return false;
    }
    // LAN→WAN handover (TRANSPORT.md § Asymmetric mux): libp2p stays connected on a dead
    // direct LAN mux while the peer is on mobile-data. Must not treat this as stable — that
    // skips coord lookup and leaves chat broken until mDNS Expired (minutes).
    if !peer_has_live_mdns_lan(session, peer) && session.peer_has_direct_connection(peer) {
        return false;
    }
    if session.dm_link_needs_recovery(peer, now_ms) {
        return false;
    }
    if let Some(pk) = pk_hex {
        if session.is_pk_reconnect_urgent(pk, now_ms) {
            return false;
        }
    }
    false
}

/// dm_upkeep coord loop — skip when stable mux **and** relay link exist (parallel LAN+WAN).
fn coord_lookup_upkeep_satisfied(
    swarm: &Swarm<ChatBehaviour>,
    session: &SessionState,
    peer: PeerId,
    pk: &str,
    now_ms: i64,
) -> bool {
    // Foreground / intent peer during LAN handover — must coord-lookup even if conn=true (07:23 logs).
    if session.lan_listen_rediscovery_requested(peer)
        && (session.is_foreground_peer(peer)
            || session.has_pending_outbox_for_pk(pk)
            || session.peer_has_pending_outbound_blockers(peer))
    {
        return false;
    }
    if peer_wan_asymmetric_mux_likely(session, peer) {
        return false;
    }
    if peer_needs_zombie_mux_reopen(session, peer) {
        return false;
    }
    if dm_peer_chat_link_stable(swarm, session, peer, Some(pk), now_ms) {
        if crate::coord_runtime::coord_is_configured() && !session.peer_has_relay_connection(peer) {
            return false;
        }
        return true;
    }
    // Relay hop up but chat mux down — intent peers must enter the lookup/reopen path.
    // (Returning true here skipped recovery while outbox resync ran into a dead mux.)
    if peer_wan_relay_connected(swarm, session, peer) {
        if peer_has_stale_direct_lan_conn(session, peer) {
            return false;
        }
        if !session.dm_peer_stream_up(peer)
            && (session.peer_has_pending_outbox(peer)
                || session.is_foreground_peer(peer)
                || session.is_peer_reconnect_urgent(peer, now_ms)
                || session.has_pending_read_acks_for(peer))
        {
            return false;
        }
        return true;
    }
    false
}

fn coord_dial_from_lookup_addrs(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    target: PeerId,
    addrs: Vec<Multiaddr>,
    now_ms: i64,
    wan_additive: bool,
    urgent: bool,
) {
    if addrs.is_empty() {
        return;
    }
    // One relay circuit per contact — attach/reopen stream on the existing hop (TRANSPORT.md § one mux).
    if peer_wan_relay_connected(swarm, session, target) {
        if !session.dm_peer_stream_up(target) {
            session.request_dm_stream_reopen(target);
        }
        return;
    }
    let needs_relay_dial = addrs
        .iter()
        .any(crate::p2p::network_transport::is_relay_circuit_multiaddr);
    if needs_relay_dial && !own_bootstrap_ready_for_peer_relay_dial(session) {
        if session.should_log_dial_skip(target, now_ms, 8_000) {
            native_log::info(
                "coord",
                format!(
                    "skip peer relay dial {target}: own bootstrap TCP not up — finish relay reservation first"
                ),
            );
        }
        return;
    }
    // LAN→WAN: close dead direct mux before coord relay dial (desktop Wi‑Fi while peer on cell).
    if !peer_has_live_mdns_lan(session, target)
        && swarm.is_connected(&target)
        && session.peer_has_direct_connection(target)
    {
        apply_peer_left_local_lan(swarm, session, target);
    }
    let wan_additive_now =
        swarm.is_connected(&target) && crate::coord_runtime::coord_is_configured();
    if dm_peer_chat_link_stable(swarm, session, target, None, now_ms)
        && !needs_additive_relay_dial(session, target, wan_additive_now)
    {
        return;
    }
    let mut ranked = sort_dm_dial_addrs_for_profile(session, target, addrs, true);
    if let Some(lan_ma) = session.peer_mdns_lan_addr(target) {
        if is_tcp_multiaddr(&lan_ma)
            && !crate::p2p::network_transport::is_relay_circuit_multiaddr(&lan_ma)
        {
            ranked.retain(|ma| ma != &lan_ma);
            ranked.insert(0, lan_ma);
        }
    }
    // On Wi‑Fi/LAN skip stale coord RFC1918 when live mDNS has a candidate — except during
    // handover when mDNS was purged and the peer has not re-announced yet (Android OEM flaps).
    if session.network_profile_snapshot().has_active_lan() {
        let allow_coord_lan = session.peer_mdns_lan_addr(target).is_none()
            && session.lan_listen_rediscovery_requested(target);
        if !allow_coord_lan {
            ranked.retain(|ma| !is_direct_lan_tcp_ma(ma));
        }
    }
    // Additive WAN while direct LAN is up: dial relay circuit only (direct already connected).
    if wan_additive && session.peer_has_direct_connection(target) {
        ranked.retain(|ma| crate::p2p::network_transport::is_relay_circuit_multiaddr(ma));
    }
    // Mobile-data and off-LAN WAN peers: relay circuit only (coord may still list stale LAN TCP).
    if session.prefers_mobile_coord_strategy()
        || (crate::coord_runtime::coord_is_configured()
            && !peer_expects_lan_discovery(session, target))
    {
        ranked.retain(|ma| crate::p2p::network_transport::is_relay_circuit_multiaddr(ma));
        if ranked.is_empty() {
            return;
        }
    }
    let Some(ma) = ranked.into_iter().next() else {
        return;
    };
    if wan_additive || (swarm.is_connected(&target) && crate::p2p::network_transport::is_relay_circuit_multiaddr(&ma)) {
        if urgent || session.should_routed_dial(target, now_ms, 2_000) {
            dial_additive_dm_addr(swarm, session, target, ma, "coord-additive");
        }
    } else {
        let tag = if is_direct_lan_tcp_ma(&ma) {
            "lan"
        } else {
            "coord"
        };
        let before = session
            .circuit_coord_dial_last_ms
            .read()
            .ok()
            .and_then(|m| m.get(&target).copied())
            .unwrap_or(0);
        dial_dm_peer_addr(swarm, session, target, ma, tag);
        let after = session
            .circuit_coord_dial_last_ms
            .read()
            .ok()
            .and_then(|m| m.get(&target).copied())
            .unwrap_or(0);
        if after != before && tag == "coord" {
            native_log::info(
                "coord",
                format!("coord_lookup_peer ok — dialing {target} via relay circuit"),
            );
        }
    }
}

/// Decide whether `pk` needs a coord lookup and, if so, **request** one in the background. This is
/// synchronous and never touches coord HTTP itself — it only inspects swarm/session state and kicks
/// [`request_coord_lookup`]. The actual dial happens later in [`apply_coord_lookup_result`] once the
/// background fetch lands. Keeping HTTP off the swarm loop is mandatory (AGENTS.md golden rule 9).
fn coord_lookup_dm_peer(
    swarm: &Swarm<ChatBehaviour>,
    session: &SessionState,
    public_key_hex: &str,
) {
    let pk = public_key_hex.trim();
    if pk.len() != 66 {
        return;
    }
    let now_ms = chrono_now_ms();
    let target = peer_id_from_secp256k1_public_key_hex(pk)
        .ok()
        .and_then(|s| s.parse::<PeerId>().ok());
    let Some(target) = target else {
        return;
    };
    let connected = swarm.is_connected(&target);
    if session.circuit_dial_in_flight_blocks(target, now_ms) {
        if peer_connect_trace_enabled(session, target)
            && session.should_log_dial_skip(target, now_ms, 5_000)
        {
            native_log::debug(
                "coord",
                format!("lookup skip {target} — relay circuit dial in flight"),
            );
        }
        return;
    }
    if peer_wan_relay_connected(swarm, session, target)
        && !peer_has_stale_direct_lan_conn(session, target)
    {
        if !session.dm_peer_stream_up(target)
            && session.peer_has_pending_wire_work(target)
        {
            notify_stream_reopen();
        }
        return;
    }
    // Stable chat mux on an existing link — skip coord churn unless WAN relay is not up yet.
    if dm_peer_chat_link_stable(swarm, session, target, Some(pk), now_ms) {
        let needs_relay = crate::coord_runtime::coord_is_configured()
            && !session.peer_has_relay_connection(target);
        if !needs_relay {
            if peer_connect_trace_enabled(session, target)
                && session.should_log_dial_skip(target, now_ms, 5_000)
            {
                native_log::debug(
                    "coord",
                    format!("lookup skip {target} — chat link already stable (relay + mux up)"),
                );
            }
            return;
        }
    }
    // Active intent — an urgent window (explicit send / recent drop), pending outbox, or the
    // foreground chat — bypasses the 404 backoff: intent beats backoff (TRANSPORT.md § prime
    // directive, acceptance criterion #4). A peer last seen "absent" is exactly the peer the user
    // wants to reach; if it is reachable now we must find it within seconds. Idle peers (no intent)
    // still respect the backoff so a genuinely-offline contact is not hammered.
    let intent = session.is_pk_reconnect_urgent(pk, now_ms)
        || session.has_pending_outbox_for_pk(pk)
        || live_foreground_peer().as_deref().is_some_and(|f| f.eq_ignore_ascii_case(pk));
    // Parallel LAN+WAN: when libp2p-connected (including direct LAN), still coord-lookup + additive relay.
    let wan_additive = connected && crate::coord_runtime::coord_is_configured();
    if connected && !wan_additive {
        return;
    }
    let skip_coord_http = !intent && session.should_skip_coord_lookup_pk(pk, now_ms);
    if skip_coord_http
        && peer_connect_trace_enabled(session, target)
        && session.should_log_dial_skip(target, now_ms, 5_000)
    {
        native_log::debug(
            "coord",
            format!(
                "lookup skip {target} — coord HTTP throttled (404/unreachable backoff); \
                 will retry after backoff or on urgent reconnect"
            ),
        );
    }
    if !skip_coord_http {
        request_coord_lookup(pk);
    }
}

/// Apply a completed background coord lookup: do the same bookkeeping (backoff/category) and dial
/// decision the old inline path did, but synchronously on the swarm loop. Swarm state is re-checked
/// here because the lookup ran across at least one tick (the peer may have connected meanwhile).
fn apply_coord_lookup_result(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    pk: &str,
    outcome: CoordLookupOutcome,
    now_ms: i64,
) {
    let pk = pk.trim();
    if pk.len() != 66 {
        return;
    }
    let Some(target) = peer_id_from_secp256k1_public_key_hex(pk)
        .ok()
        .and_then(|s| s.parse::<PeerId>().ok())
    else {
        return;
    };
    match outcome {
        CoordLookupOutcome::Ok(addrs) => {
            session.clear_coord_lookup_backoff(pk);
            session.set_coord_lookup_category(
                pk,
                crate::p2p::connectivity_diag::CoordLookupCategory::Ok,
            );
            let addrs = if crate::coord_runtime::coord_is_configured()
                && session.prefers_mobile_coord_strategy()
            {
                crate::p2p::network_transport::wan_coord_dial_addrs(addrs)
            } else {
                addrs
            };
            if addrs.is_empty() {
                session.set_coord_lookup_category(
                    pk,
                    crate::p2p::connectivity_diag::CoordLookupCategory::NoDialableAddrs,
                );
                let (reason, action) = crate::p2p::connectivity_diag::explain_coord_lookup_failure(
                    crate::p2p::connectivity_diag::CoordLookupCategory::NoDialableAddrs,
                    crate::coord_runtime::coord_is_registered(),
                );
                native_log::warn(
                    "coord",
                    format!(
                        "lookup {pk} — {reason} | next={action} | raw=no dialable addrs in presence record"
                    ),
                );
                return;
            }
            let dial_now = chrono_now_ms();
            let wan_additive_now =
                swarm.is_connected(&target) && crate::coord_runtime::coord_is_configured();
            if peer_wan_relay_connected(swarm, session, target) {
                if !session.dm_peer_stream_up(target) {
                    session.request_dm_stream_reopen(target);
                }
                return;
            }
            if dm_peer_chat_link_stable(swarm, session, target, Some(pk), dial_now)
                && !needs_additive_relay_dial(session, target, wan_additive_now)
            {
                return;
            }
            let urgent_now = session.is_pk_reconnect_urgent(pk, dial_now);
            coord_dial_from_lookup_addrs(
                swarm,
                session,
                target,
                addrs,
                dial_now,
                wan_additive_now,
                urgent_now,
            );
        }
        CoordLookupOutcome::Err(es) => {
            let cat = crate::p2p::connectivity_diag::classify_coord_lookup_error(&es);
            session.set_coord_lookup_category(pk, cat);
            if cat == crate::p2p::connectivity_diag::CoordLookupCategory::PeerNotOnCoord {
                session.note_coord_lookup_not_found(pk, now_ms);
            } else if cat == crate::p2p::connectivity_diag::CoordLookupCategory::CoordHttpUnreachable
            {
                session.note_coord_lookup_http_unreachable(pk, now_ms);
                crate::coord_runtime::note_coord_transport_failure();
            } else {
                crate::coord_runtime::note_coord_transport_failure();
            }
            let (reason, action) = crate::p2p::connectivity_diag::explain_coord_lookup_failure(
                cat,
                crate::coord_runtime::coord_is_registered(),
            );
            let log_line = format!(
                "lookup {pk} — category={} | reason={reason} | next={action} | http={es}",
                cat.as_str()
            );
            if cat == crate::p2p::connectivity_diag::CoordLookupCategory::PeerNotOnCoord
                && !session.should_log_coord_lookup_info(
                    pk,
                    now_ms,
                    COORD_PEER_NOT_ON_COORD_LOG_MIN_MS,
                )
            {
                native_log::debug("coord", log_line);
            } else {
                native_log::info("coord", log_line);
            }
        }
    }
}

/// One scale-safe coordination-lookup pass (TRANSPORT.md § "Instant connect at any roster size").
///
/// Shared by the ~1s upkeep tick and the post-handover / post-WAN-recovery burst so both honour
/// the same guarantee: **a reachable peer connects within seconds no matter how many stale
/// contacts exist.** Peers with *active intent* are looked up uncapped every pass; idle contacts
/// (possibly thousands of offline/404 entries) are swept LRU under a per-pass cap so they can
/// never flood coord or push a live peer to the back of a long sequential `await` chain:
///
/// - **urgent** — connection just dropped; reconnect bypasses the 404 backoff (uncapped).
/// - **priority** — pending outbox or the foreground chat (uncapped).
/// - **background** — everyone else, sorted oldest-looked-up first, at most
///   `COORD_BACKGROUND_LOOKUPS_PER_TICK` per pass.
///
/// `force_wake` (used by the recovery burst) makes eligible peers look up immediately instead of
/// waiting for their per-peer min interval; it does **not** lift the background cap.
fn run_dm_coord_lookup_pass(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    now_ms: i64,
    force_wake: bool,
) {
    // First, apply any background lookups that completed since the last pass. This is the dial side
    // of the split: the HTTP fetch ran off the swarm loop (request_coord_lookup) so libp2p stayed
    // responsive; here we consume the result and dial synchronously. Done before the bootstrap
    // deferral below so ready relay-circuit addrs are not dropped — coord_dial_from_lookup_addrs
    // re-checks own_bootstrap readiness per peer.
    for (pk, outcome) in drain_ready_coord_lookups(now_ms) {
        apply_coord_lookup_result(swarm, session, &pk, outcome, now_ms);
    }
    // Peer relay circuits cancel each other (oneshot) while our bootstrap TCP is still down.
    if crate::coord_runtime::coord_is_configured()
        && !own_bootstrap_ready_for_peer_relay_dial(session)
        && !relay_circuit_listening(swarm)
    {
        if session.should_log_dial_skip(bootstrap_defer_log_peer(), now_ms, 8_000) {
            native_log::info(
                "coord",
                "lookup pass deferred — own bootstrap TCP not up (WAN path first)",
            );
        }
        return;
    }
    let mut lk_urgent = 0usize;
    let mut lk_priority = 0usize;
    let mut bg_eligible = 0usize;
    let mut bg_swept = 0usize;
    // Urgent (explicit send / recent drop) — uncapped; bypasses the 404 backoff for a bounded
    // window. Intent beats backoff (TRANSPORT.md § prime directive): do **not** skip an urgent peer
    // just because it was last seen "absent" — that is precisely the peer the user wants to reach.
    for pk in session.urgent_reconnect_pks(now_ms) {
        if let Ok(derived) = peer_id_from_secp256k1_public_key_hex(&pk) {
            if let Ok(peer) = derived.parse::<PeerId>() {
                if coord_lookup_upkeep_satisfied(swarm, session, peer, &pk, now_ms) {
                    continue;
                }
            }
        }
        coord_lookup_dm_peer(swarm, session, &pk);
        lk_urgent += 1;
    }
    if crate::coord_runtime::coord_is_configured() {
        let fg = live_foreground_peer();
        let mut background: Vec<String> = Vec::new();
        for pk in session.dm_public_keys() {
            if session.is_pk_reconnect_urgent(&pk, now_ms) {
                continue; // handled by the uncapped urgent loop above
            }
            let Some(target) = peer_id_from_secp256k1_public_key_hex(&pk)
                .ok()
                .and_then(|s| s.parse::<PeerId>().ok())
            else {
                continue;
            };
            if coord_lookup_upkeep_satisfied(swarm, session, target, &pk, now_ms) {
                continue; // already has a usable relay/direct path
            }
            // Active intent (pending outbox or foreground chat) is classified BEFORE the 404
            // backoff check, and bypasses it (`should_coord_lookup_intent_pk`): a peer the user is
            // trying to reach must connect within seconds even if it was last seen offline — see
            // TRANSPORT.md § prime directive, acceptance criterion #4 "Intent beats backoff".
            let is_priority = session.pending_outbox_eligible_for_wire(&pk, now_ms)
                || fg.as_deref().is_some_and(|f| f.eq_ignore_ascii_case(&pk));
            if is_priority {
                if force_wake
                    || session.should_coord_lookup_intent_pk(
                        &pk,
                        now_ms,
                        DM_COORD_LOOKUP_MIN_INTERVAL_MS,
                    )
                {
                    coord_lookup_dm_peer(swarm, session, &pk);
                    lk_priority += 1;
                }
                continue;
            }
            // Idle (no active intent): the 404 backoff applies and the LRU cap bounds the sweep.
            if session.should_skip_coord_lookup_pk(&pk, now_ms) {
                continue; // 404 / unreachable peer inside its backoff window
            }
            bg_eligible += 1;
            background.push(pk);
        }
        // Bounded, fair (LRU) idle sweep: oldest-looked-up contacts first so a huge stale roster
        // is covered gradually without flooding coord or starving peers late in the list.
        if !background.is_empty() {
            background.sort_by_key(|pk| session.coord_lookup_last_ms(pk));
            let cap = COORD_BACKGROUND_LOOKUPS_PER_TICK.min(background.len());
            for pk in background.iter().take(cap) {
                if force_wake
                    || session.should_coord_lookup_pk(pk, now_ms, DM_COORD_LOOKUP_MIN_INTERVAL_MS)
                {
                    coord_lookup_dm_peer(swarm, session, pk);
                    bg_swept += 1;
                }
            }
        }
    }
    // Precise per-pass flow (no wrong impression): exactly what the coord phase did, including how
    // large the idle roster is and that it is swept, not ignored.
    if lk_urgent + lk_priority + bg_swept > 0 || bg_eligible > COORD_BACKGROUND_LOOKUPS_PER_TICK {
        native_log::debug(
            "coord",
            format!(
                "lookup pass: urgent={lk_urgent} priority={lk_priority} \
                 bg_swept={bg_swept}/{bg_eligible} cap={COORD_BACKGROUND_LOOKUPS_PER_TICK} \
                 force_wake={force_wake}"
            ),
        );
    }
}

