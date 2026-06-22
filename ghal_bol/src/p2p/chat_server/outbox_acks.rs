fn sync_outbox_from_transcript(session: &SessionState, path: &Path, app_namespace: &str) -> usize {
    let Ok(rows) = crate::dm_transcript_v1::pending_outbound_rows(path, app_namespace) else {
        return 0;
    };
    let mut merged = 0usize;
    for row in rows {
        if merge_outbound_row_into_outbox(session, &row) {
            merged += 1;
        }
    }
    merged
}

/// Drop in-memory outbox rows when the transcript no longer lists them as pending.
fn purge_outbox_delivered_from_transcript(
    session: &SessionState,
    path: &Path,
    app_namespace: &str,
) {
    let Ok(rows) = crate::dm_transcript_v1::pending_outbound_rows(path, app_namespace) else {
        return;
    };
    let still_pending: HashSet<String> = rows.into_iter().map(|r| r.message_id).collect();
    let Ok(mut g) = session.outbox.write() else {
        return;
    };
    let before = g.len();
    g.retain(|id, _| still_pending.contains(id));
    let removed = before.saturating_sub(g.len());
    if removed > 0 {
        native_log::debug(
            "outbox",
            format!(
                "purged {removed} delivered row(s) from in-memory outbox (transcript authoritative)"
            ),
        );
    }
}

/// Upkeep tick: sync in-memory outbox from transcript, then purge delivered rows.
fn transcript_sync_outbound_tick(session: &SessionState, path: &Path, app_namespace: &str) {
    sync_outbox_from_transcript(session, path, app_namespace);
    purge_outbox_delivered_from_transcript(session, path, app_namespace);
}

fn bootstrap_outbox_from_transcript(session: &SessionState, path: &Path, app_namespace: &str) {
    let restored = sync_outbox_from_transcript(session, path, app_namespace);
    if restored > 0 {
        native_log::info(
            "outbox",
            format!("restored {restored} pending outbound row(s) from transcript"),
        );
        // Pending outbox must drive WAN reconnect in `:p2p` — never wait for the user to open a
        // room or send a new message (DESIGN.md: background node owns delivery).
        // Do not mark ghost 404 contacts urgent — outbox waits for dm_upkeep discovery.
        for pk in session.dm_public_keys() {
            if !session.has_pending_outbox_for_pk(&pk) {
                continue;
            }
            if session.coord_lookup_category_for_pk(&pk)
                == Some(crate::p2p::connectivity_diag::CoordLookupCategory::PeerNotOnCoord)
            {
                continue;
            }
            session.refresh_dm_reconnect_urgent(&pk);
        }
        notify_coord_lookup();
    }
}

fn seed_read_acks_for_peer_from_transcript(session: &SessionState, peer: PeerId) {
    let ns = session
        .app_namespace
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty());
    let Some(ns) = ns else {
        return;
    };
    let path_buf = session
        .transcript_path
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(Path::new)
        .map(|p| p.to_path_buf())
        .or_else(|| crate::dm_transcript_store::resolve_transcript_path(ns).ok());
    let Some(path_buf) = path_buf else {
        return;
    };
    let Some(dm) = session.dm_peer_for_libp2p(peer) else {
        return;
    };
    let Some(signing) = dm.public_key_hex.as_deref() else {
        return;
    };
    let lookup_keys = crate::dm_event_handler::inbound_transcript_lookup_keys(
        ns,
        signing,
        signing,
        &peer.to_string(),
    );
    let key_set: std::collections::HashSet<String> = lookup_keys.into_iter().collect();
    let Ok(rows) =
        crate::dm_transcript_v1::pending_inbound_read_ack_rows(path_buf.as_path(), ns)
    else {
        return;
    };
    let mut seeded = 0usize;
    for row in rows {
        if !key_set.contains(row.conversation_key.as_str()) {
            continue;
        }
        if session.is_read_ack_confirmed(&row.message_id) {
            continue;
        }
        session.enqueue_read_ack(peer, &row.message_id, signing);
        seeded += 1;
    }
    if seeded > 0 {
        native_log::info(
            "read_ack",
            format!("seeded {seeded} pending read ack(s) for {peer} from transcript"),
        );
    }
}

/// Delivery ack for inbound text — sent even when the user is outside the chat room.
async fn send_inbound_delivery_ack(
    peer: PeerId,
    inbound_id: &str,
    sender_signing: &str,
    session: &SessionState,
    writers: &StreamWriters,
) {
    if session.is_delivery_ack_sent(inbound_id) {
        return;
    }
    if send_ack_frame(
        peer,
        sender_signing,
        inbound_id,
        MsgKind::AckReceived,
        session,
        writers,
    )
    .await
    {
        session.mark_delivery_ack_sent(inbound_id);
        session.dequeue_delivery_ack(inbound_id);
        native_log::info(
            "delivery_ack",
            format!("ack_received sent for inbound {inbound_id} to {peer}"),
        );
        return;
    }
    session.enqueue_delivery_ack(peer, inbound_id, sender_signing);
    native_log::warn(
        "delivery_ack",
        format!("ack_received queued for inbound {inbound_id} to {peer} (stream not ready)"),
    );
}

/// Enqueue + send `ack_read` (caller gates: only for in-room arrivals).
async fn send_inbound_read_ack_if_possible(
    peer: PeerId,
    inbound_id: &str,
    sender_signing: &str,
    session: &SessionState,
    writers: &StreamWriters,
) {
    if session.is_read_ack_confirmed(inbound_id) {
        return;
    }
    if !session.try_claim_read_ack_wire_send(peer, inbound_id, sender_signing) {
        return;
    }
    if send_ack_frame(
        peer,
        sender_signing,
        inbound_id,
        MsgKind::AckRead,
        session,
        writers,
    )
    .await
    {
        session.mark_read_ack_wire_sent(inbound_id);
        native_log::info(
            "read_ack",
            format!("ack_read sent for inbound {inbound_id} to {peer}"),
        );
        return;
    }
    session.release_read_ack_wire_claim(inbound_id);
    native_log::warn(
        "read_ack",
        format!("ack_read queued for {inbound_id} to {peer} (stream not ready)"),
    );
}

async fn send_ack_frame(
    peer: PeerId,
    recipient_signing: &str,
    ref_id: &str,
    kind: MsgKind,
    session: &SessionState,
    writers: &StreamWriters,
) -> bool {
    let Ok(env) = build_ack_envelope(
        &new_msg_id(),
        ref_id,
        kind,
        session.identity.keypair(),
        recipient_signing,
        chrono_now_ms(),
    ) else {
        return false;
    };
    let Ok(frame) = envelope_to_frame_bytes(&env) else {
        return false;
    };
    send_frame_to_peer(peer, frame, Arc::clone(writers), Some(session))
        .await
        .is_ok()
}

/// Retry queued delivery + read acks. Delivery always; queued `ack_read` always (UI background ok).
async fn run_ack_upkeep(
    session: Arc<SessionState>,
    writers: StreamWriters,
    connected_peers: &[PeerId],
) {
    if connected_peers.is_empty() {
        return;
    }
    run_ack_upkeep_limited(
        session,
        writers,
        connected_peers,
        READ_ACK_UPKEEP_MAX_OPS_PER_TICK,
        true,
    )
    .await;
}

/// Fast path for delivery acks only (~25 ms poll tick). Read ack retries use ~1 s upkeep.
async fn run_delivery_ack_upkeep(
    session: Arc<SessionState>,
    writers: StreamWriters,
    connected_peers: &[PeerId],
) {
    if connected_peers.is_empty() {
        return;
    }
    run_ack_upkeep_limited(
        session,
        writers,
        connected_peers,
        READ_ACK_UPKEEP_MAX_OPS_PER_TICK,
        false,
    )
    .await;
}

async fn run_ack_upkeep_limited(
    session: Arc<SessionState>,
    writers: StreamWriters,
    connected_peers: &[PeerId],
    read_limit: usize,
    include_read_acks: bool,
) -> usize {
    if connected_peers.is_empty() || read_limit == 0 {
        return 0;
    }
    let connected: HashSet<PeerId> = connected_peers.iter().copied().collect();
    let mut done = 0usize;
    // Delivery acks always retry (background, UI lock, read gate must not block these).
    let delivery_batch = session.delivery_acks_due_for_upkeep(read_limit);
    for item in delivery_batch {
        if done >= read_limit {
            break;
        }
        if !connected.contains(&item.peer_id) || !writer_open_for_peer(&writers, item.peer_id) {
            continue;
        }
        if send_ack_frame(
            item.peer_id,
            &item.recipient_public_key_hex,
            &item.inbound_id,
            MsgKind::AckReceived,
            session.as_ref(),
            &writers,
        )
        .await
        {
            session.mark_delivery_ack_sent(&item.inbound_id);
            session.dequeue_delivery_ack(&item.inbound_id);
            done += 1;
        }
    }
    // Queued `ack_read` retries run in :p2p even when the UI is backgrounded; only *new* in-room
    // arrivals are gated by `app_ack_read_enabled` + foreground peer (see inbound text handler).
    if include_read_acks {
        let read_batch = session.read_acks_due_for_upkeep(read_limit.saturating_sub(done));
        for item in read_batch {
            if session.is_read_ack_confirmed(&item.inbound_id) {
                continue;
            }
            if !connected.contains(&item.peer_id) || !writer_open_for_peer(&writers, item.peer_id) {
                continue;
            }
            if send_ack_frame(
                item.peer_id,
                &item.recipient_public_key_hex,
                &item.inbound_id,
                MsgKind::AckRead,
                session.as_ref(),
                &writers,
            )
            .await
            {
                session.mark_read_ack_wire_sent(&item.inbound_id);
                done += 1;
            }
        }
    }
    done
}

/// Burst-send queued `ack_read`. When [seed_transcript] is true (enter room), seed transcript
/// then drain. When false (leave room), drain only — caller seeds transcript synchronously first.
async fn read_ack_catchup_for_peer(
    session: Arc<SessionState>,
    writers: StreamWriters,
    peer: PeerId,
    wait_for_writer: bool,
    seed_transcript: bool,
) {
    if seed_transcript && !may_send_in_room_read_ack(session.as_ref(), peer) {
        return;
    }
    if seed_transcript {
        seed_read_acks_for_peer_from_transcript(session.as_ref(), peer);
    }
    if wait_for_writer {
        for _ in 0..80 {
            if writer_open_for_peer(&writers, peer) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
    if writer_open_for_peer(&writers, peer) {
        run_ack_upkeep_burst(session, writers, peer).await;
    }
}

/// After [chat_ready] / foreground peer: drain pending `ack_read` quickly (bounded).
async fn run_ack_upkeep_burst(session: Arc<SessionState>, writers: StreamWriters, peer: PeerId) {
    let connected = [peer];
    for round in 0..ACK_BURST_MAX_ROUNDS {
        let pending_before = session.pending_read_ack_len();
        let n = run_ack_upkeep_limited(
            Arc::clone(&session),
            Arc::clone(&writers),
            &connected,
            ACK_BURST_MAX_OPS_PER_PASS,
            true,
        )
        .await;
        if n == 0 && session.pending_read_ack_len() == pending_before {
            break;
        }
        if round + 1 >= ACK_BURST_MAX_ROUNDS {
            break;
        }
    }
}

/// Flush queued call signaling once the DM stream to a peer is up.
async fn flush_pending_call_signals(
    session: Arc<SessionState>,
    writers: StreamWriters,
    connected_peers: Vec<PeerId>,
    events_tx: Option<std::sync::mpsc::Sender<GossipChatEvent>>,
) {
    let batch = session.drain_pending_call_signals(32);
    if batch.is_empty() {
        return;
    }
    for call in batch {
        if call.signal_kind == CallSigKind::Invite {
            if !call_invite_is_live(call.created_at_ms, chrono_now_ms()) {
                native_log::info(
                    "call",
                    format!(
                        "drop stale queued invite call_id={} age_ms={}",
                        call.call_id,
                        chrono_now_ms().saturating_sub(call.created_at_ms)
                    ),
                );
                continue;
            }
            if !call_state::outbound_invite_active(&call.recipient_public_key_hex, &call.call_id) {
                native_log::info(
                    "call",
                    format!(
                        "drop queued invite call_id={} — call already ended",
                        call.call_id
                    ),
                );
                continue;
            }
        }
        if !connected_peers.iter().any(|id| *id == call.peer_id) {
            session.requeue_pending_call_signal_front(call);
            continue;
        }
        if !writer_open_for_peer(&writers, call.peer_id) {
            session.requeue_pending_call_signal_front(call);
            continue;
        }
        let peer_id = call.peer_id;
        let signal_kind = call.signal_kind;
        let call_id_log = call.call_id.clone();
        let recipient_pk = call.recipient_public_key_hex.clone();
        match send_frame_to_peer(
            peer_id,
            call.frame.clone(),
            Arc::clone(&writers),
            Some(session.as_ref()),
        )
        .await
        {
            Ok(()) => {
                native_log::info(
                    "call",
                    format!(
                        "call frame on wire peer={peer_id} {} call_id={call_id_log}",
                        signal_kind.wire_name(),
                    ),
                );
                if let Some(tx) = events_tx.as_ref() {
                    let _ = tx.send(GossipChatEvent::CallSignalSent {
                        call_id: call_id_log,
                        signal: signal_kind.wire_name().to_string(),
                        recipient_public_key_hex: recipient_pk,
                    });
                }
            }
            Err(e) => {
                if is_transient_outbound_error(&e) {
                    session.requeue_pending_call_signal_front(call);
                } else {
                    native_log::warn(
                        "call",
                        format!(
                            "call send failed peer={} {} call_id={}: {e}",
                            call.peer_id,
                            call.signal_kind.wire_name(),
                            call.call_id
                        ),
                    );
                    if let Some(tx) = &events_tx {
                        let _ = tx.send(GossipChatEvent::DialFailed {
                            peer: Some(call.peer_id),
                            error: e,
                        });
                    }
                }
            }
        }
    }
}

