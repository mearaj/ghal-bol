fn emit_chat_ready_if_can_send(
    session: Arc<SessionState>,
    peer: PeerId,
    writers: StreamWriters,
    events_tx: Option<std::sync::mpsc::Sender<GossipChatEvent>>,
) {
    if !session.is_dm_contact(peer) || !writer_open_for_peer(&writers, peer) {
        return;
    }
    session.ensure_dm_peer_from_libp2p(peer);
    if let (Some(path), Some(ns)) = (&session.transcript_path, &session.app_namespace) {
        transcript_sync_outbound_tick(session.as_ref(), Path::new(path), ns.trim());
    }
    let first = session
        .chat_ready_emitted
        .write()
        .ok()
        .is_some_and(|mut g| g.insert(peer));
    let has_outbox = peer_has_pending_outbox(session.as_ref(), peer);
    if !first && !has_outbox {
        return;
    }
    if first {
        // Flow milestone at info (App-log visible): this is the moment chat actually works —
        // connection alone (`conn=true`) is not enough, the `/ghal-bol/msg` stream must be open.
        // Once-per-peer-per-stream (gated by `first`), so no log spam. See AGENTS.md § "conn=true
        // ≠ chat works" and TRANSPORT.md § "Logging — see the precise flow".
        native_log::info(
            "stream",
            format!("chat_ready {peer} — chat stream open, can send now (outbox_pending={has_outbox})"),
        );
        if let Some(pk) = session.signing_pk_for_libp2p_peer(peer) {
            if !session.has_pending_outbox_for_pk(&pk) {
                session.clear_dm_reconnect_urgent(&pk);
            }
        }
        if let Some(tx) = events_tx.clone() {
            let _ = tx.send(GossipChatEvent::ChatReady { peer_id: peer });
        }
    }
    let session2 = Arc::clone(&session);
    let writers2 = Arc::clone(&writers);
    tokio::spawn(async move {
        maybe_send_transport_kem_hello(session2.as_ref(), peer, &writers2);
        // Burst before the ~1s periodic resync in this task: backlog must drain on stream-open
        // without waiting for upkeep (DESIGN.md — :p2p owns background delivery). Running periodic
        // resync first marks rows on_wire within OUTBOX_RESEND_INTERVAL_MS and the burst would
        // skip them even when the peer never got the frame (TRANSPORT.md § outbox burst ordering).
        resync_outbox_burst_for_peer(
            session2.clone(),
            writers2.clone(),
            peer,
            events_tx.clone(),
            None,
        )
        .await;
        resync_pending_outbox(
            session2.clone(),
            writers2.clone(),
            vec![peer],
            events_tx.clone(),
            None,
        )
        .await;
        flush_pending_call_signals(
            session2.clone(),
            Arc::clone(&writers2),
            vec![peer],
            events_tx.clone(),
        )
        .await;
        // Drain in-room read-ack backlog once the mux is live. Do not seed transcript here —
        // handover must not mark unread mail read; only rows already queued at room enter/leave.
        if session2.has_pending_read_acks_for(peer)
            && may_wire_read_ack_upkeep(session2.as_ref(), peer)
        {
            run_ack_upkeep_burst(session2.clone(), writers2.clone(), peer).await;
        }
        // Read receipts: only on room enter (`RunReadAckCatchup` / `SetForegroundPeer`), inbound
        // text while in-room, leave drain, and ack upkeep — never on automatic stream reopen
        // after network handover (would mark unread mail read on the sender).
        // Transcript-authoritative fallback when in-memory outbox missed rows (peer-key race on
        // stream open, stale merge). Once per connection; no room/foreground gate (DESIGN.md).
        let first_replay = session2
            .history_replay_done
            .write()
            .ok()
            .is_some_and(|mut g| g.insert(peer));
        if first_replay
            && session2
                .signing_pk_for_libp2p_peer(peer)
                .is_some_and(|pk| session2.has_pending_outbox_for_pk(&pk))
        {
            replay_conversation_history(session2.clone(), writers2, peer).await;
        }
        if let Some(pk) = session2.signing_pk_for_libp2p_peer(peer) {
            if let Some(ns) = session2
                .app_namespace
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                let _ = crate::contacts_v1::refresh_thread_preview_from_transcript(
                    ns,
                    &pk,
                    Some(&peer.to_string()),
                );
            }
        }
    });
}

/// After connect, resend transcript rows still marked pending (does not flood delivered history).
async fn replay_conversation_history(
    session: Arc<SessionState>,
    writers: StreamWriters,
    peer: PeerId,
) {
    let (path, ns) = match (&session.transcript_path, &session.app_namespace) {
        (Some(p), Some(n)) if !p.trim().is_empty() && !n.trim().is_empty() => {
            (p.clone(), n.trim().to_string())
        }
        _ => return,
    };
    if !writer_open_for_peer(&writers, peer) {
        return;
    }
    let recipient_pk = match session.signing_pk_for_libp2p_peer(peer) {
        Some(pk) => pk,
        None => return,
    };
    let Ok(rows) = crate::dm_transcript_v1::pending_outbound_rows(Path::new(&path), &ns) else {
        return;
    };
    let peer_s = peer.to_string();
    let mut sent = 0usize;
    for row in rows {
        let ck = row.conversation_key.as_str();
        if ck != peer_s && ck != recipient_pk {
            continue;
        }
        if !writer_open_for_peer(&writers, peer) {
            break;
        }
        let pending = PendingOutbound {
            message_id: row.message_id.clone(),
            peer_id: peer,
            recipient_public_key_hex: recipient_pk.to_string(),
            text: row.text.clone(),
            created_at_ms: if row.created_at_ms > 0 {
                row.created_at_ms
            } else {
                chrono_now_ms()
            },
            last_send_ms: chrono_now_ms(),
            first_on_wire_ms: 0,
            on_wire: false,
        };
        let Ok(frame) = build_pending_outbound_frame(session.as_ref(), &pending) else {
            continue;
        };
        if send_frame_to_peer(peer, frame, Arc::clone(&writers), Some(session.as_ref()))
            .await
            .is_ok()
        {
            sent += 1;
        }
        tokio::time::sleep(Duration::from_millis(HISTORY_REPLAY_SPACING_MS)).await;
    }
    if sent > 0 {
        native_log::info(
            "history",
            format!("replayed {sent} pending outbound line(s) to {peer}"),
        );
    }
}

fn writer_open_for_peer(writers: &StreamWriters, peer: PeerId) -> bool {
    writers.lock().ok().is_some_and(|g| g.contains_key(&peer))
}

/// Drop the mux writer and schedule reopen — must clear both the flag and the writers map.
pub(crate) fn invalidate_dm_chat_stream(
    session: &SessionState,
    writers: &StreamWriters,
    peer: PeerId,
) {
    if let Ok(mut g) = writers.lock() {
        g.remove(&peer);
    }
    session.set_dm_stream_writer(peer, false);
    session.clear_chat_ready_emitted(peer);
    if let Ok(mut g) = session.stream_open_inflight.write() {
        g.remove(&peer);
    }
    notify_stream_reopen();
}

pub(crate) fn spawn_leave_read_ack_drain(
    session: Arc<SessionState>,
    writers: StreamWriters,
    left: PeerId,
    control: stream::Control,
) {
    let cutoff = read_ack_cutoff_ms(session.as_ref(), left);
    let pk_label = session
        .signing_pk_for_libp2p_peer(left)
        .unwrap_or_else(|| left.to_string());
    native_log::info(
        "read_ack",
        format!(
            "chat room leave {pk_label} — drain ack_read cutoff_ms={cutoff} (new mail: recv only)"
        ),
    );
    dispatch_read_ack_pass(session, writers, left, cutoff, true, Some(control));
}

fn is_transient_outbound_error(err: &str) -> bool {
    let e = err.to_lowercase();
    e.contains("connecting to peer")
        || e.contains("writer wait timed out")
        || e.contains("chat stream not ready")
        || e.contains("wait until connected")
        || e.contains("open_stream")
        || e.contains("broken pipe")
        || e.contains("connection reset")
        || e.contains("stream closed")
        || e.contains("transport kem not ready")
}

fn is_transport_kem_deferred(err: &str) -> bool {
    err.to_lowercase().contains("transport kem not ready")
}

fn notify_outbound_on_wire(
    session: &SessionState,
    message_id: &str,
    now_ms: i64,
    events_tx: &Option<std::sync::mpsc::Sender<GossipChatEvent>>,
) {
    if !session.mark_outbox_sent(message_id, now_ms) {
        return;
    }
    if let Some(tx) = events_tx {
        let _ = tx.send(GossipChatEvent::OutboundSent {
            message_id: message_id.to_string(),
        });
    }
}

fn send_frame_on_open_stream(
    peer: PeerId,
    frame: Vec<u8>,
    writers: &StreamWriters,
) -> Result<(), String> {
    queue_frame_on_open_stream(peer, frame, writers, None)
}

fn queue_frame_on_open_stream(
    peer: PeerId,
    frame: Vec<u8>,
    writers: &StreamWriters,
    written: Option<tokio::sync::oneshot::Sender<bool>>,
) -> Result<(), String> {
    let tx = {
        let g = writers
            .lock()
            .map_err(|_| "writers mutex poisoned".to_string())?;
        g.get(&peer).cloned()
    };
    let Some(tx) = tx else {
        return Err("no chat stream to peer yet — wait until connected".to_string());
    };
    tx.send(StreamWireItem::Frame {
        bytes: frame,
        written,
    })
    .map_err(|_| "chat stream closed".to_string())
}

