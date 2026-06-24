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

/// Live LAN reachability — mDNS candidate only (not `peers_on_local_lan` TTL stamps).
fn peer_has_live_mdns_lan(session: &SessionState, peer: PeerId) -> bool {
    session.peer_mdns_lan_addr(peer).is_some()
}

/// Parallel LAN+WAN: stream is up on a live path but relay hop is missing — pursue throttled
/// additive relay dial without treating the chat mux as down (TRANSPORT.md § Both links active).
fn needs_additive_relay_dial(session: &SessionState, peer: PeerId, connected: bool) -> bool {
    connected
        && crate::coord_runtime::coord_is_configured()
        && !session.peer_has_relay_connection(peer)
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
    // Peer is reachable on our LAN right now → parallel LAN+WAN is intended; not a stale mux.
    if peer_has_live_mdns_lan(session, peer) {
        return false;
    }
    let has_relay = session.peer_has_relay_connection(peer);
    let has_direct = peer_has_stale_direct_lan_conn(session, peer);
    // No live mDNS but a direct LAN mux lingers next to relay — asymmetric handover: the peer
    // went to WAN/mobile-data; recover the relay mux for acks/outbox (close direct, keep relay).
    if has_relay && has_direct {
        return true;
    }
    !has_relay
}

fn peer_has_stale_direct_lan_conn(session: &SessionState, peer: PeerId) -> bool {
    session
        .dm_direct_conn_ids
        .read()
        .ok()
        .is_some_and(|m| m.get(&peer).is_some_and(|s| !s.is_empty()))
}

/// Do not open a chat stream on a dead direct LAN mux while WAN relay recovery is pending.
fn should_defer_stream_open_for_wan_mux(session: &SessionState, peer: PeerId) -> bool {
    if !dm_peer_needs_wan_relay_path(session, peer) {
        return false;
    }
    if peer_has_stale_direct_lan_conn(session, peer) {
        return true;
    }
    !session.peer_has_relay_connection(peer)
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
        // Stale LAN-only mux while peer is on mobile-data — must coord-dial relay (ticks/acks).
        if dm_peer_needs_wan_relay_path(session, peer) {
            return false;
        }
        return true;
    }
    if !swarm.is_connected(&peer) {
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
    session
        .chat_ready_emitted
        .read()
        .ok()
        .is_some_and(|g| g.contains(&peer))
}

/// dm_upkeep coord loop — skip when stable mux **and** relay link exist (parallel LAN+WAN).
fn coord_lookup_upkeep_satisfied(
    swarm: &Swarm<ChatBehaviour>,
    session: &SessionState,
    peer: PeerId,
    pk: &str,
    now_ms: i64,
) -> bool {
    if !dm_peer_chat_link_stable(swarm, session, peer, Some(pk), now_ms) {
        return false;
    }
    if crate::coord_runtime::coord_is_configured() && !session.peer_has_relay_connection(peer) {
        return false;
    }
    true
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

async fn coord_lookup_dm_peer(
    swarm: &mut Swarm<ChatBehaviour>,
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
        match crate::coord_runtime::lookup_dial_multiaddrs_for_public_key_async(pk).await {
            Ok(addrs) => {
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
                } else {
                    let dial_now = chrono_now_ms();
                    let wan_additive_now = swarm.is_connected(&target)
                        && crate::coord_runtime::coord_is_configured();
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
            }
            Err(e) => {
                let es = e.to_string();
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
async fn run_dm_coord_lookup_pass(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    now_ms: i64,
    force_wake: bool,
) {
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
        coord_lookup_dm_peer(swarm, session, &pk).await;
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
            let is_priority = session.has_pending_outbox_for_pk(&pk)
                || fg.as_deref().is_some_and(|f| f.eq_ignore_ascii_case(&pk));
            if is_priority {
                if force_wake
                    || session.should_coord_lookup_intent_pk(
                        &pk,
                        now_ms,
                        DM_COORD_LOOKUP_MIN_INTERVAL_MS,
                    )
                {
                    coord_lookup_dm_peer(swarm, session, &pk).await;
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
                    coord_lookup_dm_peer(swarm, session, pk).await;
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

