/// Resolve which connected libp2p peer should receive this outbox row (match by secp256k1 pk, not stale peer_id).
fn outbox_target_peer(
    session: &SessionState,
    p: &PendingOutbound,
    connected_peers: &[PeerId],
) -> Option<PeerId> {
    let pk = p.recipient_public_key_hex.trim();
    if pk.len() == 66 {
        for id in connected_peers {
            if session
                .dm_peer_for_libp2p(*id)
                .and_then(|d| d.public_key_hex.clone())
                .is_some_and(|k| k.eq_ignore_ascii_case(pk))
            {
                return Some(*id);
            }
        }
        if let Some(id) = session.resolve_send_peer(pk) {
            if connected_peers.contains(&id) {
                return Some(id);
            }
        }
    }
    if connected_peers.contains(&p.peer_id) {
        Some(p.peer_id)
    } else {
        None
    }
}

fn refresh_outbox_peer_ids(session: &SessionState) {
    let Ok(mut g) = session.outbox.write() else {
        return;
    };
    for p in g.values_mut() {
        let pk = p.recipient_public_key_hex.trim();
        if pk.len() == 66 {
            if let Some(id) = session.resolve_send_peer(pk) {
                p.peer_id = id;
            }
        }
    }
}

/// Resend outbound texts until `ack_received` or `ack_read` (~1s cadence).
///
/// The **transcript** is authoritative; call [`transcript_sync_outbound_tick`] before this on every tick.
async fn resync_pending_outbox(
    session: Arc<SessionState>,
    writers: StreamWriters,
    connected_peers: Vec<PeerId>,
    events_tx: Option<std::sync::mpsc::Sender<GossipChatEvent>>,
    control: Option<stream::Control>,
) {
    refresh_outbox_peer_ids(session.as_ref());
    let now = chrono_now_ms();
    // Only rows whose recipient is actually connected right now can go on the wire. Filtering here
    // (instead of iterating the whole outbox) stops the misleading "resync N pending" storm and
    // wasted frame builds for the thousands of offline/ghost contacts whose rows never send — the
    // real wire send already no-ops for them. This is connectivity, not coord policy: a peer that
    // is connected still drains regardless of any stale 404 coord category.
    let due: Vec<_> = session
        .outbox_due_for_resend(now)
        .into_iter()
        .filter(|p| outbox_target_peer(session.as_ref(), p, &connected_peers).is_some())
        .collect();
    if !due.is_empty() {
        native_log::debug("outbox", format!("resync {} pending message(s)", due.len()));
    }
    for p in due {
        let Some(send_peer) = outbox_target_peer(session.as_ref(), &p, &connected_peers) else {
            continue;
        };
        if !writer_open_for_peer(&writers, send_peer) {
            if let Some(ctrl) = control.as_ref() {
                ensure_dm_chat_stream(
                    send_peer,
                    Arc::clone(&session),
                    Arc::clone(&writers),
                    ctrl.clone(),
                    events_tx.clone(),
                )
                .await;
            }
            if !writer_open_for_peer(&writers, send_peer) {
                continue;
            }
        }
        let frame = match build_pending_outbound_frame(session.as_ref(), &p) {
            Ok(f) => f,
            Err(e) => {
                native_log::warn(
                    "outbox",
                    format!("resync skip msg_id={}: {e}", p.message_id),
                );
                continue;
            }
        };
        match send_frame_to_peer(
            send_peer,
            frame,
            Arc::clone(&writers),
            Some(session.as_ref()),
        )
        .await
        {
            Ok(()) => {
                session.mark_outbox_sent(&p.message_id, now);
                notify_outbound_on_wire(&session, &p.message_id, now, &events_tx);
            }
            Err(e) => {
                session.mark_outbox_send_failed(&p.message_id, now);
                invalidate_dm_chat_stream(session.as_ref(), &writers, send_peer);
                native_log::debug(
                    "outbox",
                    format!("resync send failed msg_id={}: {e}", p.message_id),
                );
            }
        }
    }
}

/// On chat stream open: drain every pending row for this peer immediately (TRANSPORT.md — outbox must not wait on timers).
async fn resync_outbox_burst_for_peer(
    session: Arc<SessionState>,
    writers: StreamWriters,
    peer: PeerId,
    events_tx: Option<std::sync::mpsc::Sender<GossipChatEvent>>,
    control: Option<stream::Control>,
) {
    refresh_outbox_peer_ids(session.as_ref());
    let Some(pk) = session.signing_pk_for_libp2p_peer(peer) else {
        native_log::info(
            "outbox",
            format!("burst skip {peer}: no signing pk for connected peer yet"),
        );
        return;
    };
    let now = chrono_now_ms();
    let rows: Vec<PendingOutbound> = session
        .outbox
        .read()
        .ok()
        .map(|g| {
            g.values()
                .filter(|p| {
                    p.recipient_public_key_hex.eq_ignore_ascii_case(&pk) || p.peer_id == peer
                })
                // Skip only rows already on the wire within OUTBOX_RESEND_INTERVAL_MS — not failed
                // sends (`on_wire=false`). Re-sending an in-flight row double-delivers and the peer
                // emits duplicate `ack_received` (TRANSPORT.md § Post-mortem 2026-06-25). Backlog
                // and not-yet-delivered rows drain immediately on stream-open.
                .filter(|p| {
                    !p.on_wire
                        || now.saturating_sub(p.last_send_ms) >= OUTBOX_RESEND_INTERVAL_MS
                })
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    if rows.is_empty() {
        if session.has_pending_outbox_for_pk(&pk) {
            native_log::info(
                "outbox",
                format!(
                    "burst skip {peer}: in-memory rows not eligible (on_wire/throttle) — transcript replay will retry"
                ),
            );
        }
        return;
    }
    native_log::info(
        "outbox",
        format!("burst resync {} pending row(s) to {peer}", rows.len()),
    );
    for p in rows {
        if !writer_open_for_peer(&writers, peer) {
            if let Some(ctrl) = control.as_ref() {
                ensure_dm_chat_stream(
                    peer,
                    Arc::clone(&session),
                    Arc::clone(&writers),
                    ctrl.clone(),
                    events_tx.clone(),
                )
                .await;
            }
            if !writer_open_for_peer(&writers, peer) {
                break;
            }
        }
        let frame = match build_pending_outbound_frame(session.as_ref(), &p) {
            Ok(f) => f,
            Err(e) => {
                native_log::warn("outbox", format!("burst skip msg_id={}: {e}", p.message_id));
                continue;
            }
        };
        match send_frame_to_peer(peer, frame, Arc::clone(&writers), Some(session.as_ref())).await {
            Ok(()) => {
                session.mark_outbox_sent(&p.message_id, now);
                notify_outbound_on_wire(session.as_ref(), &p.message_id, now, &events_tx);
            }
            Err(e) => {
                session.mark_outbox_send_failed(&p.message_id, now);
                invalidate_dm_chat_stream(session.as_ref(), &writers, peer);
                native_log::warn(
                    "outbox",
                    format!("burst send failed msg_id={}: {e}", p.message_id),
                );
            }
        }
    }
}

/// Connected DM contacts that need a chat stream (acks/outbox cannot send without it).
fn dm_peers_missing_chat_stream(
    swarm: &Swarm<ChatBehaviour>,
    session: &SessionState,
    writers: &StreamWriters,
) -> Vec<PeerId> {
    session
        .dm_peer_ids()
        .into_iter()
        .filter(|pid| {
            swarm.is_connected(pid)
                && !writer_open_for_peer(writers, *pid)
                && !should_defer_stream_open_for_wan_mux(session, *pid)
                && !should_defer_outbound_stream_for_asymmetric_relay(session, *pid)
        })
        .collect()
}

fn spawn_reopen_dm_chat_streams(
    swarm: &Swarm<ChatBehaviour>,
    session: Arc<SessionState>,
    writers: StreamWriters,
    control: stream::Control,
    events_tx: Option<std::sync::mpsc::Sender<GossipChatEvent>>,
) {
    for pid in dm_peers_missing_chat_stream(swarm, session.as_ref(), &writers) {
        let session2 = Arc::clone(&session);
        let writers2 = Arc::clone(&writers);
        let events_tx2 = events_tx.clone();
        let control2 = control.clone();
        tokio::spawn(async move {
            ensure_dm_chat_stream(pid, session2, writers2, control2, events_tx2).await;
        });
    }
}

/// Drop a stale LAN-only mux when the peer needs a relay circuit (asymmetric LAN↔WAN handover).
fn reconcile_all_stale_lan_mux_for_wan(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    writers: &StreamWriters,
) {
    for peer in session.dm_peer_ids() {
        reconcile_stale_lan_mux_for_wan(swarm, session, writers, peer);
    }
}

fn wan_mux_reconcile_throttle_mx() -> &'static RwLock<HashMap<PeerId, i64>> {
    static M: OnceLock<RwLock<HashMap<PeerId, i64>>> = OnceLock::new();
    M.get_or_init(|| RwLock::new(HashMap::new()))
}

const WAN_MUX_RECONCILE_THROTTLE_MS: i64 = 5_000;

fn wan_mux_reconcile_throttled(peer: PeerId, now_ms: i64) -> bool {
    let Ok(mut m) = wan_mux_reconcile_throttle_mx().write() else {
        return false;
    };
    let last = m.get(&peer).copied().unwrap_or(0);
    if now_ms.saturating_sub(last) < WAN_MUX_RECONCILE_THROTTLE_MS {
        return true;
    }
    m.insert(peer, now_ms);
    false
}

/// Outbound must be stuck this long before a recently-active, writer-open mux is reconciled —
/// matches `coord_lookup::LAN_HANDOVER_STUCK_MS`. Healthy chat drains far faster, so this guards
/// against tearing down a live LAN mux mid-chat (flutter_linux.log 2026-06-28 bursty-WAN churn).
const WAN_MUX_RECONCILE_STUCK_MS: i64 = 4_000;

/// Drop a stale LAN-only mux when the peer needs a relay circuit (asymmetric LAN↔WAN handover).
fn reconcile_stale_lan_mux_for_wan(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    writers: &StreamWriters,
    peer: PeerId,
) {
    let asymmetric = peer_wan_asymmetric_mux_likely(session, peer);
    if !dm_peer_needs_wan_relay_path(session, peer) && !asymmetric {
        return;
    }
    if !swarm.is_connected(&peer) && !session.dm_peer_stream_up(peer) {
        return;
    }
    let stale_direct = peer_has_stale_direct_lan_conn(session, peer);
    let mux_reopen = peer_needs_wan_mux_reopen(session, peer);
    if !stale_direct && !mux_reopen {
        return;
    }
    let now_ms = chrono_now_ms();
    let urgent_reconcile = session.take_asymmetric_relay_recover_urgent(peer);
    if !urgent_reconcile && wan_mux_reconcile_throttled(peer, now_ms) {
        return;
    }
    // Peer still on LAN with a healthy mux — clear spurious soft-nudge flag; do not WAN-reconcile.
    if session.network_profile_snapshot().has_active_lan()
        && (peer_has_live_mdns_lan(session, peer) || session.peer_on_local_lan(peer))
        && session.dm_peer_stream_up(peer)
        && session.dm_mux_recently_active(peer, now_ms)
        && !session.peer_outbound_stuck_for(peer, now_ms, WAN_MUX_RECONCILE_STUCK_MS)
    {
        session.clear_lan_listen_rediscovery(peer);
        return;
    }
    if urgent_reconcile {
        // Stamp throttle after one-shot bypass — no reconcile storm (TRANSPORT.md § recovery).
        if let Ok(mut m) = wan_mux_reconcile_throttle_mx().write() {
            m.insert(peer, now_ms);
        }
    }
    // Bidirectional mux with a live writer — do not tear down mid-flight. Inbound-only
    // activity on a read-only duplicate stream must not block asymmetric LAN↔WAN recovery.
    // "Stuck" is sustained (LAN_HANDOVER_STUCK_MS), not a frame momentarily in flight: a healthy
    // LAN mux that is actively draining outbound must survive (flutter_linux.log 2026-06-28 churn).
    if writer_open_for_peer(writers, peer)
        && session.dm_mux_recently_active(peer, now_ms)
        && !session.peer_outbound_stuck_for(peer, now_ms, WAN_MUX_RECONCILE_STUCK_MS)
    {
        return;
    }
    session.clear_peer_stale_lan_cache(peer);

    if stale_direct && session.peer_has_relay_connection(peer) {
        native_log::info(
            "stream",
            format!("close stale direct {peer} — chat stream up, relay kept"),
        );
        close_direct_dm_connections(swarm, session, peer);
        if !mux_reopen {
            // Stuck outbound on a lingering direct mux → writer is zombie; full invalidate per
            // TRANSPORT.md § Asymmetric mux recovery. Healthy relay-only path keeps flag-only
            // reopen (Symptom C — do not tear down a live LAN mux).
            if session.peer_outbound_stuck_for(peer, now_ms, WAN_MUX_RECONCILE_STUCK_MS)
                && writer_open_for_peer(writers, peer)
            {
                invalidate_dm_chat_stream(session, writers, peer);
            } else {
                session.request_dm_stream_reopen(peer);
            }
            notify_coord_lookup();
            return;
        }
    }

    if mux_reopen || (stale_direct && !session.peer_has_relay_connection(peer)) {
        native_log::info(
            "stream",
            format!("reopen {peer} — peer off LAN; recover WAN mux for acks/outbox"),
        );
        invalidate_dm_chat_stream(session, writers, peer);
        if session.peer_has_relay_connection(peer) {
            if stale_direct {
                close_direct_dm_connections(swarm, session, peer);
            }
            session.request_dm_stream_reopen(peer);
        } else {
            session.request_dm_link_reset(peer);
            if let Some(pk) = session
                .dm_peer_for_libp2p(peer)
                .and_then(|d| d.public_key_hex.clone())
            {
                session.mark_dm_reconnect_urgent(&pk);
            }
        }
        notify_coord_lookup();
    }
}

/// Replace a dead writer while `stream=true` — invalidate + reopen only; no close-direct (TRANSPORT.md).
fn reopen_zombie_dm_mux_if_needed(
    session: &SessionState,
    writers: &StreamWriters,
    peer: PeerId,
) {
    if !peer_needs_zombie_mux_reopen(session, peer) {
        return;
    }
    let now_ms = chrono_now_ms();
    if wan_mux_reconcile_throttled(peer, now_ms) {
        return;
    }
    // Mux exchanged frames recently on a relay-only path — not a zombie writer yet.
    if session.dm_mux_recently_active(peer, now_ms)
        && !peer_has_stale_direct_lan_conn(session, peer)
        && !peer_wan_asymmetric_mux_likely(session, peer)
    {
        return;
    }
    native_log::info(
        "stream",
        format!(
            "reopen {peer} — outbound on wire stuck; replace dead mux writer"
        ),
    );
    invalidate_dm_chat_stream(session, writers, peer);
    session.request_dm_stream_reopen(peer);
    if session.peer_has_pending_wire_work(peer) || session.is_foreground_peer(peer) {
        if let Some(pk) = session
            .dm_peer_for_libp2p(peer)
            .and_then(|d| d.public_key_hex.clone())
        {
            session.mark_dm_reconnect_urgent(&pk);
        }
    }
    notify_coord_lookup();
}

fn reopen_all_zombie_dm_mux(session: &SessionState, writers: &StreamWriters) {
    for peer in session.dm_peer_ids() {
        reopen_zombie_dm_mux_if_needed(session, writers, peer);
    }
}

/// DM upkeep: stream up → noop; else unified connect (LAN mDNS → coord).
fn upkeep_dm_peers(
    swarm: &mut Swarm<ChatBehaviour>,
    session: Arc<SessionState>,
    control: stream::Control,
    writers: StreamWriters,
    events_tx: Option<std::sync::mpsc::Sender<GossipChatEvent>>,
) {
    let mut peers: Vec<PeerId> = session.dm_peer_ids();
    peers.sort_by_key(|p| (!session.peer_has_pending_wire_work(*p), *p));
    for peer in peers {
        if session.dm_peer_stream_up(peer) {
            if !swarm.is_connected(&peer) {
                invalidate_dm_chat_stream(session.as_ref(), &writers, peer);
                continue;
            }
            if peer_needs_zombie_mux_reopen(session.as_ref(), peer) {
                reopen_zombie_dm_mux_if_needed(session.as_ref(), &writers, peer);
            } else if asymmetric_relay_recover_on_existing_link(session.as_ref(), peer) {
                reconcile_stale_lan_mux_for_wan(swarm, session.as_ref(), &writers, peer);
            }
            continue;
        }
        if swarm.is_connected(&peer) {
            if asymmetric_relay_recover_on_existing_link(session.as_ref(), peer) {
                reconcile_stale_lan_mux_for_wan(swarm, session.as_ref(), &writers, peer);
            }
            if should_defer_stream_open_for_wan_mux(session.as_ref(), peer) {
                if peer_connect_trace_enabled(session.as_ref(), peer)
                    && session.should_log_dial_skip(peer, chrono_now_ms(), 5_000)
                {
                    native_log::debug(
                        "stream",
                        format!(
                            "stream open deferred {peer} — connected on a transient mux; \
                             waiting for stable WAN relay mux before opening chat stream"
                        ),
                    );
                }
                continue;
            }
            if should_defer_outbound_stream_for_asymmetric_relay(session.as_ref(), peer) {
                if peer_connect_trace_enabled(session.as_ref(), peer)
                    && session.should_log_dial_skip(peer, chrono_now_ms(), 5_000)
                {
                    native_log::debug(
                        "stream",
                        format!(
                            "stream open deferred {peer} — asymmetric relay on LAN; \
                             waiting for peer inbound stream"
                        ),
                    );
                }
                continue;
            }
            if !writer_open_for_peer(&writers, peer) {
                let session2 = Arc::clone(&session);
                let writers2 = Arc::clone(&writers);
                let events_tx2 = events_tx.clone();
                let control2 = control.clone();
                tokio::spawn(async move {
                    ensure_dm_chat_stream(peer, session2, writers2, control2, events_tx2).await;
                });
            }
            continue;
        }
        reconcile_lan_dial_in_flight(swarm, session.as_ref(), peer);
        connect_dm_peer_now(swarm, session.as_ref(), peer);
    }
}

/// Skip coord **relay** dials only when a relay link already carries a stable DM stream.
fn skip_redundant_coord_relay_dial(
    swarm: &Swarm<ChatBehaviour>,
    session: &SessionState,
    peer: PeerId,
    now_ms: i64,
) -> bool {
    if !session.peer_has_relay_connection(peer) {
        return false;
    }
    if !swarm.is_connected(&peer) {
        return false;
    }
    if session.dm_link_needs_recovery(peer, now_ms) {
        return false;
    }
    let pk = session
        .dm_peer_for_libp2p(peer)
        .and_then(|d| d.public_key_hex.clone());
    dm_peer_chat_link_stable(swarm, session, peer, pk.as_deref(), now_ms)
}

fn peer_has_pending_outbox(session: &SessionState, peer: PeerId) -> bool {
    session.peer_has_pending_outbox(peer)
}

fn dm_connect_is_urgent(session: &SessionState, peer: PeerId, now_ms: i64) -> bool {
    if session.is_peer_reconnect_urgent(peer, now_ms) {
        return true;
    }
    if session.peer_has_pending_wire_work(peer) {
        return true;
    }
    if session.network_profile_snapshot().has_active_lan() && session.peer_has_pending_outbox(peer) {
        return true;
    }
    if !session.peer_has_pending_outbox(peer) {
        return false;
    }
    let Some(pk) = session
        .dm_peer_for_libp2p(peer)
        .and_then(|d| d.public_key_hex.clone())
    else {
        return true;
    };
    !session.peer_coord_absent_never_connected(&pk, peer)
}

/// True while our LAN dial window for this peer has not expired — **never** call `swarm.dial(peer_id)` to probe (side-effect: blind peerstore multi-dial).
fn lan_dial_still_in_flight(session: &SessionState, peer: PeerId, now_ms: i64) -> bool {
    session
        .lan_dial_in_flight_start_ms(peer)
        .is_some_and(|start| now_ms.saturating_sub(start) < LAN_DIAL_IN_FLIGHT_MS)
}

/// LAN dial window ended — kick WAN coord lookup in parallel with any ongoing mDNS recovery.
fn lan_dial_expired_coord_fallback(
    _swarm: &mut Swarm<ChatBehaviour>,
    _session: &SessionState,
    peer: PeerId,
) {
    native_log::info(
        "mdns",
        format!("LAN dial window expired for {peer} — coord/WAN fallback (parallel with mDNS)"),
    );
    notify_dm_presence_wake();
    if crate::coord_runtime::coord_is_configured() {
        notify_coord_lookup();
    }
}

fn sync_wan_coord_local_snapshot(swarm: &Swarm<ChatBehaviour>) {
    crate::wan_coord::sync_local_relay_circuit_listening(relay_circuit_listening(swarm));
}

/// Apply [`wan_coord::WanCoordEffect`] from the WAN coordination hub (TRANSPORT.md § WAN coordination).
fn apply_wan_coord_effects(
    effects: &[crate::wan_coord::WanCoordEffect],
    relay_peer: Option<PeerId>,
    session: Option<&SessionState>,
) {
    for effect in effects {
        match effect {
            crate::wan_coord::WanCoordEffect::MarkRelayHopLost => {
                crate::coord_runtime::mark_coord_relay_hop_lost();
            }
            crate::wan_coord::WanCoordEffect::ScheduleCoordPresenceAfterRelay => {
                crate::coord_runtime::schedule_coord_presence_after_relay();
            }
            crate::wan_coord::WanCoordEffect::ScheduleCoordPresencePoll => {
                crate::coord_runtime::ensure_coord_presence_polling();
            }
            crate::wan_coord::WanCoordEffect::NotifyRelayRefresh => notify_relay_refresh(),
            crate::wan_coord::WanCoordEffect::NotifyCoordLookup => notify_coord_lookup(),
            crate::wan_coord::WanCoordEffect::NotifyDmPresenceWake => notify_dm_presence_wake(),
            crate::wan_coord::WanCoordEffect::MarkRemoteCircuitStale(pk) => {
                crate::coord_runtime::note_remote_peer_circuit_stale(pk);
            }
            crate::wan_coord::WanCoordEffect::NoteRelayReservation => {
                if let Some(relay) = relay_peer {
                    crate::coord_runtime::coord_note_relay_reservation(relay);
                }
            }
            crate::wan_coord::WanCoordEffect::NotifyStreamReopen => notify_stream_reopen(),
            crate::wan_coord::WanCoordEffect::MarkAllDmReconnectUrgent => {
                if let Some(session) = session {
                    session.mark_dm_reconnect_urgent_unless_live_direct_stream();
                }
            }
        }
    }
}

fn is_bare_p2p_peer_multiaddr(ma: &Multiaddr, peer: PeerId) -> bool {
    is_bare_peer_multiaddr(ma)
        && ma
            .iter()
            .any(|p| matches!(p, Protocol::P2p(id) if id == peer))
}

fn dm_connection_is_relay(
    session: &SessionState,
    peer: PeerId,
    endpoint: &ConnectedPoint,
) -> bool {
    let remote = endpoint.get_remote_address();
    // Parallel LAN+WAN: LAN TCP win must not inherit relay classification from an in-flight circuit dial.
    if is_direct_lan_tcp_ma(remote) {
        session.clear_relay_circuit_pending_peer(peer);
        return false;
    }
    if crate::p2p::network_transport::is_relay_circuit_multiaddr(remote) {
        session.clear_relay_circuit_pending_peer(peer);
        return true;
    }
    // Relay v2 circuits often report as `/p2p/<peer>` without `/p2p-circuit` (#5741).
    if session.take_relay_circuit_pending_peer(peer) {
        return true;
    }
    // Real LAN direct always carries `/ip4/…/tcp/…`; bare `/p2p/<peer>` is relay misreport.
    if is_bare_p2p_peer_multiaddr(remote, peer) && crate::coord_runtime::coord_is_configured() {
        return true;
    }
    false
}

fn reconcile_lan_dial_in_flight(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    peer: PeerId,
) {
    let now_ms = chrono_now_ms();
    let Some(start) = session.lan_dial_in_flight_start_ms(peer) else {
        return;
    };
    if swarm.is_connected(&peer) {
        session.clear_lan_dial_in_flight(peer);
        return;
    }
    if now_ms.saturating_sub(start) < LAN_DIAL_PENDING_GRACE_MS {
        return;
    }
    if now_ms.saturating_sub(start) >= LAN_DIAL_IN_FLIGHT_MS {
        session.clear_lan_dial_in_flight(peer);
        if !swarm.is_connected(&peer) {
            if let Some(addr) = session.peer_mdns_lan_addr(peer) {
                if dm_connect_is_urgent(session, peer, now_ms) {
                    session.remove_mdns_lan_candidate(peer, Some(&addr));
                    if let Some(next) = session.peer_mdns_lan_addr(peer) {
                        native_log::info(
                            "mdns",
                            format!(
                                "LAN dial window expired for {peer} — failover to next mDNS candidate"
                            ),
                        );
                        dial_mdns_lan_addr(swarm, session, peer, next);
                    } else {
                        session.remove_mdns_lan_candidate(peer, Some(&addr));
                        lan_dial_expired_coord_fallback(swarm, session, peer);
                    }
                } else {
                    lan_dial_expired_coord_fallback(swarm, session, peer);
                }
            } else {
                lan_dial_expired_coord_fallback(swarm, session, peer);
            }
        }
        return;
    }
    if lan_dial_still_in_flight(session, peer, now_ms) {
        return;
    }
    session.clear_lan_dial_in_flight(peer);
}

/// True for peers the user is actively trying to reach right now (foreground chat or queued
/// outbox). Per-peer connect tracing is gated on this so a huge idle roster never floods the log,
/// while the peer you actually care about always shows its precise connect flow — including the
/// reason for every early return (TRANSPORT.md § "Logging — see the precise flow").
fn peer_connect_trace_enabled(session: &SessionState, peer: PeerId) -> bool {
    if session.peer_has_pending_outbox(peer) {
        return true;
    }
    let Some(fg) = live_foreground_peer() else {
        return false;
    };
    session
        .dm_peer_for_libp2p(peer)
        .and_then(|d| d.public_key_hex.clone())
        .is_some_and(|pk| pk.eq_ignore_ascii_case(&fg))
}

/// Unified connect: parallel WAN (coord lookup) from upkeep; LAN only from mDNS events.
fn connect_dm_peer_now(swarm: &mut Swarm<ChatBehaviour>, session: &SessionState, target: PeerId) {
    if swarm.is_connected(&target) {
        return;
    }
    let now_ms = chrono_now_ms();
    if !dm_connect_is_urgent(session, target, now_ms) {
        // Idle contacts: wait for mDNS `Discovered` — no timer LAN re-dials from candidate cache.
        if peer_connect_trace_enabled(session, target)
            && session.should_log_dial_skip(target, now_ms, 5_000)
        {
            native_log::debug(
                "dial",
                format!(
                    "connect skip {target} — idle (no pending outbox / not urgent); \
                     LAN waits for mDNS Discovered, WAN waits for coord wake"
                ),
            );
        }
        return;
    }
    if let Some(pk) = session
        .dm_peer_for_libp2p(target)
        .and_then(|d| d.public_key_hex.clone())
    {
        if !session.peer_coord_absent_never_connected(&pk, target) {
            session.mark_dm_reconnect_urgent(&pk);
        }
    }
    if crate::coord_runtime::coord_is_configured() {
        notify_coord_lookup();
    } else if session.circuit_dial_in_flight_blocks(target, now_ms) {
        if peer_connect_trace_enabled(session, target)
            && session.should_log_dial_skip(target, now_ms, 5_000)
        {
            native_log::debug(
                "dial",
                format!("connect skip {target} — relay circuit dial already in flight"),
            );
        }
        return;
    } else if !session.lan_candidates_exhausted(target)
        && !session.lan_dial_in_flight_blocks(target, now_ms)
    {
        try_routed_dial(swarm, session, target);
    }
}

/// Tear down one-sided libp2p links where `open_stream` timed out (peer never mux-ready).
fn apply_pending_dm_link_resets(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    writers: &StreamWriters,
) {
    for peer in session.take_pending_dm_link_resets() {
        if !swarm.is_connected(&peer) {
            continue;
        }
        session.clear_lan_dial_in_flight(peer);
        invalidate_dm_chat_stream(session, writers, peer);
        if session.peer_has_relay_connection(peer) {
            if peer_has_stale_direct_lan_conn(session, peer) {
                native_log::info(
                    "stream",
                    format!("reset DM link {peer} — drop direct mux; keep relay circuit"),
                );
                close_direct_dm_connections(swarm, session, peer);
            } else {
                native_log::info(
                    "stream",
                    format!("reset DM link {peer} — reopen chat stream on relay circuit"),
                );
            }
            notify_stream_reopen();
            continue;
        }
        native_log::info(
            "stream",
            format!("reset DM link {peer} — chat stream open timed out"),
        );
        session.note_disconnected(&peer);
        let _ = swarm.disconnect_peer_id(peer);
        if let Some(pk) = session
            .dm_peer_for_libp2p(peer)
            .and_then(|d| d.public_key_hex.clone())
        {
            session.mark_dm_reconnect_urgent(&pk);
        }
        notify_coord_lookup();
        notify_dm_presence_wake();
    }
}

/// Routed dial: legacy no-coord path only — with coord configured, LAN is mDNS explicit addr; WAN is live coord `/p2p-circuit` only.
fn try_routed_dial(swarm: &mut Swarm<ChatBehaviour>, session: &SessionState, peer: PeerId) {
    if crate::coord_runtime::coord_is_configured() {
        return;
    }
    try_routed_dial_impl(swarm, session, peer);
}

fn sort_dm_dial_addrs_for_profile(
    session: &SessionState,
    peer: PeerId,
    addrs: Vec<Multiaddr>,
    for_coord_path: bool,
) -> Vec<Multiaddr> {
    // Coord lookup addrs are relay circuits — always IPv4/dns4-first on every platform.
    // LAN desktop must not use LAN-first sort or undifferentiated WAN-first (IPv6 relay
    // often unroutable even when Wi‑Fi is up).
    if for_coord_path && crate::coord_runtime::coord_is_configured() {
        let has_wifi_lan_tcp = session.network_profile_snapshot().has_active_lan()
            && addrs.iter().any(|ma| {
                is_tcp_multiaddr(ma)
                    && !crate::p2p::network_transport::is_relay_circuit_multiaddr(ma)
                    && crate::p2p::network_transport::dm_dial_addr_rank(ma) == 0
            });
        if has_wifi_lan_tcp {
            return crate::p2p::network_transport::sort_dm_dial_addrs(addrs);
        }
        return crate::p2p::network_transport::wan_coord_dial_addrs(addrs);
    }
    // Rank LAN-first only when the peer has a **live** mDNS candidate right now — not the
    // `peers_on_local_lan` TTL stamp, which lingers ~3 min after mDNS `Expired` and would keep
    // ranking a stale LAN TCP addr ahead of the working WAN path (TRANSPORT.md § Ephemeral LAN
    // TCP ports; same live-mDNS rule as `dm_peer_needs_wan_relay_path`).
    let on_lan = session.peer_mdns_lan_addr(peer).is_some();
    if on_lan {
        return crate::p2p::network_transport::sort_dm_dial_addrs(addrs);
    }
    crate::p2p::network_transport::rank_dm_dial_addrs_for_peer(addrs, false)
}

/// Same as [try_routed_dial] but allowed after coord lookup miss (LAN/mDNS when coord has no record).
fn try_routed_dial_impl(swarm: &mut Swarm<ChatBehaviour>, session: &SessionState, peer: PeerId) {
    // Never peer-id-only dial when coord is configured — peerstore holds polluted identify addrs
    // (docker bridge, QUIC, fe80) and libp2p tries them all in parallel (TRANSPORT.md).
    if crate::coord_runtime::coord_is_configured() {
        return;
    }
    if !session.should_dial_libp2p_peer(peer) || peer == *swarm.local_peer_id() {
        return;
    }
    if swarm.is_connected(&peer) {
        return;
    }
    let now = chrono_now_ms();
    if !session.should_routed_dial(peer, now, 2_000) {
        return;
    }
    match swarm.dial(
        DialOpts::peer_id(peer)
            .condition(PeerCondition::DisconnectedAndNotDialing)
            .build(),
    ) {
        Ok(()) => {
            session.note_routed_dial_attempt(peer, now);
            native_log::debug("dial", format!("routed dial {peer}"));
        }
        Err(DialError::NoAddresses) => {
            let last = session.coord_lookup_category_for_peer(peer);
            let (reason, action) = crate::p2p::connectivity_diag::explain_no_dial_addrs(
                last,
                session.peer_on_local_lan(peer),
                crate::coord_runtime::coord_is_registered(),
            );
            if session.should_log_dial_skip(peer, now, 8_000) {
                native_log::warn(
                    "dial",
                    format!("issue=no_dial_addrs | peer={peer} | reason={reason} | next={action}"),
                );
            }
        }
        Err(DialError::DialPeerConditionFalse(_)) => {}
        Err(e) => native_log::debug("dial", format!("routed dial {peer}: {e}")),
    }
}

const MAX_IDENTIFY_DM_ADDRS_PER_PEER: usize = 4;

/// Merge dialable TCP listen addresses from identify into peerstore and dial.
fn ingest_identify_listen_addrs(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    peer: PeerId,
    addrs: &[Multiaddr],
    tag: &str,
) {
    if session.is_bootstrap_peer(peer) {
        return;
    }
    // Protonet: while chat stream is up, do not ingest stale identify addrs or dial.
    if session.dm_peer_stream_up(peer) {
        return;
    }
    if swarm.is_connected(&peer) && session.peer_has_direct_connection(peer) {
        return;
    }
    // Stale identify/LAN addrs must not pollute peerstore for off-LAN WAN contacts.
    if crate::coord_runtime::coord_is_configured() {
        if !session.network_profile_snapshot().has_active_lan() {
            return;
        }
        if !peer_expects_lan_discovery(session, peer) {
            return;
        }
    }
    let ranked = sort_dm_dial_addrs_for_profile(
        session,
        peer,
        addrs
            .iter()
            .filter(|a| crate::p2p::network_transport::is_dm_dial_multiaddr(a))
            .cloned()
            .collect(),
        true,
    );
    // On Wi‑Fi/LAN never dial RFC1918 from identify — same rule as coord lookup (live mDNS only).
    let ranked: Vec<Multiaddr> = if session.network_profile_snapshot().has_active_lan() {
        ranked
            .into_iter()
            .filter(|ma| !is_direct_lan_tcp_ma(ma))
            .collect()
    } else {
        ranked
    };
    let ranked: Vec<Multiaddr> = ranked
        .into_iter()
        .filter(|ma| !is_unusable_dm_dial_addr(session, peer, ma))
        .collect();
    if ranked.is_empty() {
        return;
    }
    // Single dial ownership (DESIGN.md § Stream-first symmetric connect): with coord configured,
    // LAN dialing is owned by the mDNS `Discovered` handler and WAN dialing by coord lookup.
    // Identify must **not** add a third competing `swarm.dial` for the same peer — it only signals
    // the owners so they (re)dial via their own event/throttle path. Avoids the peerstore-pollution
    // multi-dial this function used to cause. Legacy no-coord (LAN-only/dev) keeps the direct dial.
    if crate::coord_runtime::coord_is_configured() {
        native_log::debug(
            tag,
            format!(
                "identify {peer}: {} tcp listen addr(s) seen — signal owners (no identify dial)",
                ranked.len().min(MAX_IDENTIFY_DM_ADDRS_PER_PEER)
            ),
        );
        notify_dm_presence_wake();
        return;
    }
    if session.should_dial_libp2p_peer(peer) && !swarm.is_connected(&peer) {
        native_log::info(
            tag,
            format!(
                "identify {peer}: {} tcp listen addr(s) ingested",
                ranked.len().min(MAX_IDENTIFY_DM_ADDRS_PER_PEER)
            ),
        );
        if let Some(addr) = ranked.into_iter().next() {
            dial_dm_peer_addr(swarm, session, peer, addr, tag);
        }
    }
}

fn is_bare_peer_multiaddr(addr: &Multiaddr) -> bool {
    let mut it = addr.iter();
    matches!(it.next(), Some(Protocol::P2p(_))) && it.next().is_none()
}

/// Loopback / bare peer-id / docker / link-local addrs — never dial for DM (peerstore pollution).
fn is_polluted_dm_dial_multiaddr(addr: &Multiaddr) -> bool {
    if is_bare_peer_multiaddr(addr) {
        return true;
    }
    if is_quic_multiaddr(addr) {
        return true;
    }
    let s = addr.to_string();
    if s.contains("/ip6/::1/") || s.contains("/ip4/127.0.0.1/") || s.contains("/ip6/fe80:") {
        return true;
    }
    if let Some(ip) = crate::p2p::network_transport::ipv4_from_ma_str(&s) {
        if ip.is_loopback()
            || crate::p2p::network_transport::is_docker_or_link_local_ipv4(ip)
        {
            return true;
        }
    }
    false
}

/// Stale LAN / loopback peerstore addrs must not be dialed off-LAN (mobile-data WAN path).
fn is_unusable_dm_dial_addr(session: &SessionState, peer: PeerId, addr: &Multiaddr) -> bool {
    if is_polluted_dm_dial_multiaddr(addr) {
        return true;
    }
    // Off-LAN mobile-data: never dial RFC1918 from identify/peerstore — coord circuit only.
    if session.prefers_mobile_coord_strategy()
        && !session.network_profile_snapshot().has_active_lan()
        && is_direct_lan_tcp_ma(addr)
    {
        return true;
    }
    if session.prefers_mobile_coord_strategy()
        && session.peer_mdns_lan_addr(peer).is_none()
        && is_direct_lan_tcp_ma(addr)
    {
        return true;
    }
    false
}

fn dial_dm_peer_addr(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    peer: PeerId,
    addr: Multiaddr,
    tag: &str,
) {
    if !session.should_dial_libp2p_peer(peer) || peer == *swarm.local_peer_id() {
        return;
    }
    if session.is_bootstrap_peer(peer) {
        return;
    }
    // Multiple call-sites can attempt to dial the same DM peer concurrently:
    // - upkeep_dm_peers (coord/mDNS discovery)
    // - coord_lookup_dm_peer (explicit relay dial)
    // - SendText fast-path ("send queued: coord lookup")
    // - UI bursts of RegisterDmPeer / foreground changes
    //
    // Without a shared throttle, libp2p cancels earlier dials (seen as "oneshot canceled"),
    // and relays can rate-limit / reject ("resource limit exceeded"), causing multi-minute stalls.
    let now = chrono_now_ms();
    let is_relay = crate::p2p::network_transport::is_relay_circuit_multiaddr(&addr);
    let urgent = dm_connect_is_urgent(session, peer, now);
    let min_interval_ms = if tag.starts_with("coord") && is_relay {
        if urgent {
            CIRCUIT_COORD_DIAL_URGENT_MS
        } else {
            45_000
        }
    } else if urgent {
        LAN_DIAL_THROTTLE_URGENT_MS
    } else {
        2_000
    };
    if is_relay && session.relay_circuit_dial_backoff_active(peer, now) {
        if urgent {
            // Prime directive (AGENTS.md golden rule 9, TRANSPORT.md § "intent beats backoff"):
            // a foreground / pending-outbox peer must connect now. A prior relay rate-limit
            // (ResourceLimitExceeded → 90s backoff) must NOT keep an urgent circuit dial parked;
            // the 8s urgent throttle below is the only storm guard we keep.
            session.clear_relay_circuit_dial_backoff(peer);
            native_log::info(
                "coord",
                format!("relay circuit backoff cleared for {peer}: urgent intent beats backoff"),
            );
        } else {
            if session.should_log_dial_skip(peer, now, 8_000) {
                native_log::info(
                    "coord",
                    format!("skip relay circuit dial {peer}: rate-limit backoff active (not urgent)"),
                );
            }
            return;
        }
    }
    if is_relay && !urgent && skip_redundant_coord_relay_dial(swarm, session, peer, now) {
        return;
    }
    // TRANSPORT.md § circuit_dial_in_flight_ms — never clear early or disconnect while libp2p is
    // still handshaking; urgent reconnect uses coord lookup wake + 2s circuit retry, not oneshot cancel.
    if is_relay && session.circuit_dial_in_flight_blocks(peer, now) {
        if session.should_log_dial_skip(peer, now, 8_000) {
            native_log::info(
                "coord",
                format!(
                    "skip relay circuit dial {peer}: prior dial in flight (<{CIRCUIT_DIAL_IN_FLIGHT_MS}ms, urgent={urgent})"
                ),
            );
        }
        return;
    }
    if is_relay && tag.starts_with("coord") {
        if !session.should_circuit_coord_dial(peer, now, min_interval_ms) {
            if session.should_log_dial_skip(peer, now, 8_000) {
                native_log::info(
                    "coord",
                    format!(
                        "skip relay circuit dial {peer}: throttled (<{min_interval_ms}ms since last, urgent={urgent})"
                    ),
                );
            }
            return;
        }
    } else if !session.should_routed_dial(peer, now, min_interval_ms) {
        return;
    }
    // On mobile-data with coord configured, only relay-circuit dials are reliable — except on
    // LAN (mDNS) or RFC1918 while Wi‑Fi is active.
    if crate::coord_runtime::coord_is_configured() && session.prefers_mobile_coord_strategy() {
        let on_lan = session.peer_on_local_lan(peer)
            || peer_expects_lan_discovery(session, peer);
        if !is_relay && !on_lan {
            let now = chrono_now_ms();
            if session.should_log_dial_skip(peer, now, 8_000) {
                native_log::info(
                    "dial",
                    format!("skip direct dial {peer}: mobile coord strategy (addr not relay)"),
                );
            }
            return;
        }
    }
    #[cfg(target_os = "android")]
    {
        // Android swarm is TCP+noise only. Some relays advertise circuit addrs over QUIC/WebTransport;
        // dialing those cannot succeed and wastes the critical handover window.
        if is_relay && !is_tcp_multiaddr(&addr) {
            return;
        }
    }
    let loopback_coord = tag == "coord"
        && is_tcp_multiaddr(&addr)
        && crate::p2p::network_transport::ipv4_from_ma_str(&addr.to_string())
            .is_some_and(|ip| ip.is_loopback());
    // Never dial "bare" `/p2p/<peer>` multiaddrs (invalid / guaranteed to fail).
    if is_bare_peer_multiaddr(&addr) {
        let now = chrono_now_ms();
        if session.should_log_dial_skip(peer, now, 8_000) {
            native_log::info("dial", format!("skip invalid dial addr for {peer}: {addr}"));
        }
        return;
    }
    if !loopback_coord && !crate::p2p::network_transport::is_dm_dial_multiaddr(&addr) {
        return;
    }
    if is_unusable_dm_dial_addr(session, peer, &addr) {
        if is_relay && session.should_log_dial_skip(peer, now, 8_000) {
            native_log::info(
                "coord",
                format!("skip relay circuit dial {peer}: addr classified unusable ({addr})"),
            );
        }
        return;
    }
    if !is_relay && !is_tcp_multiaddr(&addr) {
        return;
    }
    let mut dial_ma = addr.clone();
    if !dial_ma.iter().any(|p| matches!(p, Protocol::P2p(_))) {
        dial_ma.push(Protocol::P2p(peer));
    }
    match swarm.dial(
        DialOpts::peer_id(peer)
            .addresses(vec![dial_ma.clone()])
            .condition(PeerCondition::Always)
            .build(),
    ) {
        Ok(()) => {
            if is_relay {
                session.mark_circuit_dial_in_flight(peer, now);
                session.note_relay_circuit_pending_peer(peer);
                if tag.starts_with("coord") {
                    session.note_circuit_coord_dial_attempt(peer, now);
                }
            } else {
                session.note_routed_dial_attempt(peer, now);
                session.mark_lan_dial_in_flight(peer, now);
            }
            native_log::info(tag, format!("dialing {peer} via {dial_ma}"));
        }
        Err(DialError::DialPeerConditionFalse(_)) => {
            // libp2p already has a dial (or connection) pending for this peer. For a relay-circuit
            // coord dial this is the silent case that hides "no circuit reqs at the relay": the
            // existing dial is parked (e.g. relay STOP to a flapping peer never completes) so we
            // never re-issue and the relay never sees a HOP. Surface it so the stall is visible.
            if is_relay && session.should_log_dial_skip(peer, now, 8_000) {
                native_log::info(
                    "coord",
                    format!(
                        "relay circuit dial {peer} not issued: libp2p reports dial/conn already pending ({dial_ma})"
                    ),
                );
            }
        }
        Err(e) => native_log::debug(tag, format!("dial {peer} {dial_ma}: {e}")),
    }
}

fn handle_mdns_discovered_list(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    list: Vec<(PeerId, Multiaddr)>,
    _emit: &mut dyn FnMut(GossipChatEvent),
) {
    let mut by_peer: HashMap<PeerId, Vec<Multiaddr>> = HashMap::new();
    for (peer, addr) in list {
        native_log::info("mdns", format!("discovered {peer} at {addr}"));
        by_peer.entry(peer).or_default().push(addr);
    }
    for (peer, addrs) in by_peer {
        if session.is_dm_contact(peer) {
            if let Some(pk) = secp256k1_public_key_hex_from_peer_id(&peer) {
                session.clear_peer_coord_absent_state(&pk);
            }
        }
        let mut new_lan_tcp = false;
        let mut dial_from_event: Option<Multiaddr> = None;
        let lan_tcp_discovered: Vec<Multiaddr> = addrs
            .iter()
            .filter(|a| is_direct_lan_tcp_mdns_candidate(a))
            .cloned()
            .collect();
        for addr in &addrs {
            if session.merge_mdns_lan_candidate(peer, addr) {
                if is_direct_lan_tcp_mdns_candidate(addr) {
                    new_lan_tcp = true;
                    // First new LAN TCP in this mDNS batch — stale ports often follow live ones.
                    if dial_from_event.is_none() {
                        dial_from_event = Some(addr.clone());
                    }
                }
            }
        }
        if !lan_tcp_discovered.is_empty() {
            session.note_peer_on_local_lan(peer);
        }
        let now = chrono_now_ms();
        if swarm.is_connected(&peer) {
            // Parallel LAN+WAN: on relay-only link, mDNS re-announce must still shift to direct TCP.
            if !session.peer_has_direct_connection(peer) && !lan_tcp_discovered.is_empty() {
                if let Some(addr) = dial_from_event.or_else(|| session.peer_mdns_lan_addr(peer)) {
                    dial_lan_upgrade(swarm, session, peer, addr);
                }
            } else {
                let skip_lan_upgrade = session.dm_has_stream_writer(peer)
                    && !session.dm_link_needs_recovery(peer, now)
                    && !new_lan_tcp;
                if (new_lan_tcp || session.dm_link_needs_recovery(peer, now))
                    && !skip_lan_upgrade
                    && !session.peer_has_direct_connection(peer)
                {
                    if let Some(addr) = dial_from_event.or_else(|| session.peer_mdns_lan_addr(peer))
                    {
                        dial_lan_upgrade(swarm, session, peer, addr);
                    }
                }
            }
        } else if session.is_dm_contact(peer) && !lan_tcp_discovered.is_empty() {
            // TRANSPORT.md: both on LAN → dial direct TCP from this mDNS event (not upkeep cache).
            if let Some(addr) = dial_from_event.or_else(|| session.peer_mdns_lan_addr(peer)) {
                dial_mdns_lan_addr(swarm, session, peer, addr);
            }
        }
    }
}

/// Direct LAN TCP from mDNS when the peer is not connected yet — parallel with in-flight WAN circuit dials.
/// Returns true when a dial was actually issued to libp2p.
fn dial_mdns_lan_addr(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    peer: PeerId,
    addr: Multiaddr,
) -> bool {
    if !session.should_dial_libp2p_peer(peer) || peer == *swarm.local_peer_id() {
        return false;
    }
    if swarm.is_connected(&peer) {
        return false;
    }
    if is_bare_peer_multiaddr(&addr)
        || !is_tcp_multiaddr(&addr)
        || !crate::p2p::network_transport::is_dm_dial_multiaddr(&addr)
    {
        return false;
    }
    let now = chrono_now_ms();
    reconcile_lan_dial_in_flight(swarm, session, peer);
    if !session.try_claim_lan_dial_slot(peer, now) {
        return false;
    }
    if !session.should_routed_dial(peer, now, 2_000) {
        session.clear_lan_dial_in_flight(peer);
        return false;
    }
    // Parallel LAN+WAN: do not clear `circuit_dial_in_flight` — WAN handshake may still be running.
    let mut dial_ma = addr.clone();
    if !dial_ma.iter().any(|p| matches!(p, Protocol::P2p(_))) {
        dial_ma.push(Protocol::P2p(peer));
    }
    match swarm.dial(
        DialOpts::peer_id(peer)
            .addresses(vec![dial_ma.clone()])
            .condition(PeerCondition::Always)
            .build(),
    ) {
        Ok(()) => {
            session.note_routed_dial_attempt(peer, now);
            native_log::info("mdns", format!("dialing {peer} via {dial_ma}"));
            true
        }
        Err(e) => {
            session.clear_lan_dial_in_flight(peer);
            native_log::debug("mdns", format!("dial {peer} {dial_ma}: {e}"));
            false
        }
    }
}

/// Dial a peer's **direct LAN** multiaddr even while a relay/circuit connection is already
/// open, so the faster LAN path is established. Uses `PeerCondition::Always` (rather than
/// the default `DisconnectedAndNotDialing`) so the dial is not refused just because a relay
/// link exists. Only direct TCP LAN addrs are dialed here — never relay circuits or bare ids.
/// Additive dial (WAN or LAN) while an existing link is open — `PeerCondition::Always`.
fn dial_additive_dm_addr(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    peer: PeerId,
    addr: Multiaddr,
    tag: &str,
) {
    if !session.should_dial_libp2p_peer(peer) || peer == *swarm.local_peer_id() {
        return;
    }
    if is_bare_peer_multiaddr(&addr) {
        return;
    }
    let is_relay = crate::p2p::network_transport::is_relay_circuit_multiaddr(&addr);
    if is_relay {
        let now = chrono_now_ms();
        if skip_redundant_coord_relay_dial(swarm, session, peer, now) {
            return;
        }
    }
    if !is_relay && !is_tcp_multiaddr(&addr) {
        return;
    }
    #[cfg(target_os = "android")]
    if is_relay && !is_tcp_multiaddr(&addr) {
        return;
    }
    if !is_relay && !crate::p2p::network_transport::is_dm_dial_multiaddr(&addr) {
        return;
    }
    let mut dial_ma = addr.clone();
    if !dial_ma.iter().any(|p| matches!(p, Protocol::P2p(_))) {
        dial_ma.push(Protocol::P2p(peer));
    }
    match swarm.dial(
        DialOpts::peer_id(peer)
            .addresses(vec![dial_ma.clone()])
            .condition(PeerCondition::Always)
            .build(),
    ) {
        Ok(()) => native_log::info(tag, format!("additive dial {peer} via {dial_ma}")),
        Err(DialError::DialPeerConditionFalse(_)) => {}
        Err(e) => native_log::debug(tag, format!("additive dial {peer} {dial_ma}: {e}")),
    }
}

fn dial_lan_upgrade(
    swarm: &mut Swarm<ChatBehaviour>,
    session: &SessionState,
    peer: PeerId,
    addr: Multiaddr,
) {
    if !session.should_dial_libp2p_peer(peer) || peer == *swarm.local_peer_id() {
        return;
    }
    if crate::p2p::network_transport::is_relay_circuit_multiaddr(&addr)
        || is_bare_peer_multiaddr(&addr)
        || !is_tcp_multiaddr(&addr)
    {
        return;
    }
    let mut dial_ma = addr.clone();
    if !dial_ma.iter().any(|p| matches!(p, Protocol::P2p(_))) {
        dial_ma.push(Protocol::P2p(peer));
    }
    match swarm.dial(
        DialOpts::peer_id(peer)
            .addresses(vec![dial_ma.clone()])
            .condition(PeerCondition::Always)
            .build(),
    ) {
        Ok(()) => native_log::info("mdns", format!("LAN upgrade dial {peer} via {dial_ma}")),
        Err(DialError::DialPeerConditionFalse(_)) => {}
        Err(e) => native_log::debug("mdns", format!("LAN upgrade dial {peer} {dial_ma}: {e}")),
    }
}

/// Periodic connectivity summary (grep `Native/flow` or `connectivity` in App log export).
fn log_connectivity_snapshot(
    swarm: &Swarm<ChatBehaviour>,
    session: &SessionState,
    writers: &StreamWriters,
) {
    let local = *swarm.local_peer_id();
    let listens: Vec<String> = session
        .published_listen_snapshot()
        .iter()
        .take(6)
        .map(|a| a.to_string())
        .collect();
    let mut dm_lines = Vec::new();
    for peer in session.dm_peer_ids() {
        let connected = swarm.is_connected(&peer);
        let stream = writer_open_for_peer(writers, peer);
        let label = secp256k1_public_key_hex_from_peer_id(&peer)
            .map(|pk| crate::flow_log::short_hex(&pk))
            .unwrap_or_else(|| peer.to_string());
        dm_lines.push(format!("{label}:conn={connected},stream={stream}"));
    }
    let coord_cfg = crate::coord_runtime::coord_is_configured();
    let coord_reg = crate::coord_runtime::coord_is_registered();
    let coord_degraded = crate::coord_runtime::coord_http_degraded();
    let dht_boot = session.any_bootstrap_connected.load(Ordering::Relaxed);
    let relay_circuit = relay_circuit_listening(swarm);
    let wan_recovery = session.wan_recovery_active.load(Ordering::Relaxed);
    let outbox_pending = session.pending_outbox_count();
    let links = session.connected_peers().len();
    let net = session.network_profile_snapshot();
    let profile = net.mode_label();
    let hint = net.dial_hint();
    let os_truth = net.os_truth_label();
    sync_wan_coord_local_snapshot(swarm);
    let wan_coord = crate::wan_coord::phase_label(
        coord_cfg,
        coord_reg,
        coord_degraded,
        dht_boot,
        relay_circuit,
    );
    native_log::info(
        "flow",
        format!(
            "connectivity local={local} profile={profile} {os_truth} hint={hint} \
             listen_addrs={} [{}] dm=[{}] active_links={links} \
             relay_circuit={relay_circuit} wan_recovery={wan_recovery} outbox_pending={outbox_pending} \
             coord_relay_tcp={dht_boot} coord_configured={coord_cfg} coord_registered={coord_reg} \
             coord_http_degraded={coord_degraded} wan_coord={wan_coord}",
            listens.len(),
            listens.join(" | "),
            dm_lines.join(" "),
        ),
    );
}

