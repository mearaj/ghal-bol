
/// Exposed for FFI so Dart can correlate delivery acks with outbound bubbles.
pub fn new_msg_id_for_ffi() -> String {
    new_msg_id()
}

async fn read_frame<R: AsyncReadExt + Unpin>(reader: &mut R) -> Result<Vec<u8>, String> {
    let mut len_buf = [0u8; 4];
    reader
        .read_exact(&mut len_buf)
        .await
        .map_err(|e| format!("read length: {e}"))?;
    let len = u32::from_le_bytes(len_buf) as usize;
    if len > 4 * 1024 * 1024 {
        return Err("frame too large".to_string());
    }
    let mut body = vec![0u8; len];
    reader
        .read_exact(&mut body)
        .await
        .map_err(|e| format!("read body: {e}"))?;
    let mut frame = Vec::with_capacity(4 + len);
    frame.extend_from_slice(&len_buf);
    frame.extend_from_slice(&body);
    Ok(frame)
}

async fn write_frame<W: AsyncWriteExt + Unpin>(writer: &mut W, frame: &[u8]) -> Result<(), String> {
    if frame.len() < 4 {
        return Err("frame too short".to_string());
    }
    writer
        .write_all(frame)
        .await
        .map_err(|e| format!("write frame: {e}"))?;
    writer.flush().await.map_err(|e| format!("flush: {e}"))?;
    Ok(())
}

async fn send_frame_to_peer(
    peer: PeerId,
    frame: Vec<u8>,
    writers: StreamWriters,
    session: Option<&SessionState>,
) -> Result<(), String> {
    let _ = session;
    send_frame_on_open_stream(peer, frame, &writers)
}

fn stream_read_is_terminal(err: &str) -> bool {
    let e = err.to_lowercase();
    e.contains("unexpected eof")
        || e.contains("early eof")
        || e.contains("connection reset")
        || e.contains("broken pipe")
        || e.contains("stream closed")
        || e.contains("closed by remote")
}

/// A live duplicate inbound stream takes over the writer only after outbound has been stuck this
/// long (or on stronger evidence: retransmit / documented asymmetric handover). Long enough to not
/// churn a healthy writer mid-send, short enough to recover one-way acks within a few ticks.
const DUPLICATE_MUX_TAKEOVER_STUCK_MS: i64 = 3_000;

/// Clear the writer slot for `peer` **only** if `generation` is still the live writer generation.
/// A stale stream handler (older generation) whose mux was replaced by adopt/reopen must not remove
/// the live writer or flip the stream flag — that race silently killed the relay writer and broke
/// return acks (TRANSPORT.md § Asymmetric LAN↔WAN mux recovery → writer generation).
fn finalize_dm_writer_if_current(
    session: &SessionState,
    writers: &StreamWriters,
    peer: PeerId,
    generation: u64,
) {
    if !session.release_dm_writer_generation_if_current(peer, generation) {
        return;
    }
    if let Ok(mut g) = writers.lock() {
        g.remove(&peer);
    }
    session.set_dm_stream_writer(peer, false);
    session.clear_chat_ready_emitted(peer);
    notify_stream_reopen();
}

fn spawn_dm_stream_write_task(
    peer: PeerId,
    mut writer: futures::io::WriteHalf<libp2p::Stream>,
    rx: mpsc::UnboundedReceiver<StreamWireItem>,
    generation: u64,
    writers: StreamWriters,
    session: Arc<SessionState>,
) -> tokio::task::JoinHandle<()> {
    let writers_w = Arc::clone(&writers);
    let session_w = Arc::clone(&session);
    tokio::spawn(async move {
        let mut rx = rx;
        while let Some(item) = rx.recv().await {
            let StreamWireItem::Frame { bytes, written } = item;
            let ok = write_frame(&mut writer, &bytes).await.is_ok();
            if ok {
                session_w.note_dm_wire_activity(peer);
            }
            if let Some(done) = written {
                let _ = done.send(ok);
            }
            if !ok {
                break;
            }
        }
        finalize_dm_writer_if_current(&session_w, &writers_w, peer, generation);
    })
}

/// Decide whether a **live duplicate** inbound stream should take over the writer slot.
///
/// Conservative on purpose: in healthy parallel LAN+WAN both links stay up and the peer rarely
/// writes on our duplicate, so this almost never fires there. It only takes over when there is
/// real evidence the currently-owned writer is dead — a retransmitted inbound (peer never got our
/// ack), the documented asymmetric LAN↔WAN handover, or outbound stuck past one resend window.
fn duplicate_mux_should_take_over(
    session: &SessionState,
    peer: PeerId,
    now_ms: i64,
    strong: bool,
) -> bool {
    if strong {
        return true;
    }
    if asymmetric_relay_recover_on_existing_link(session, peer) {
        return true;
    }
    if peer_wan_asymmetric_mux_likely(session, peer) || peer_needs_wan_mux_reopen(session, peer) {
        return true;
    }
    if peer_needs_zombie_mux_reopen(session, peer) {
        return true;
    }
    session.peer_outbound_stuck_for(peer, now_ms, DUPLICATE_MUX_TAKEOVER_STUCK_MS)
}

/// Inbound arrived on a secondary mux while another stream holds the writer — adopt this path so
/// acks/outbox use the live relay (asymmetric LAN↔WAN; TRANSPORT.md § Asymmetric mux recovery).
/// Installs a fresh writer generation so the evicted handler can never tear this writer down.
fn adopt_duplicate_mux_as_writer(
    peer: PeerId,
    spare_writer: &mut Option<futures::io::WriteHalf<libp2p::Stream>>,
    write_task: &mut Option<tokio::task::JoinHandle<()>>,
    owns_writer: &mut bool,
    my_gen: &mut u64,
    session: Arc<SessionState>,
    writers: StreamWriters,
    events_tx: Option<std::sync::mpsc::Sender<GossipChatEvent>>,
) {
    if *owns_writer {
        return;
    }
    let Some(writer) = spare_writer.take() else {
        return;
    };
    native_log::info(
        "stream",
        format!("inbound on duplicate mux from {peer} — adopt live writer (stale mux replaced)"),
    );
    session.clear_relay_inbound_handover_peer(peer);
    // Evict the stale writer (drops its mpsc tx → its write task ends; generation guard stops that
    // task from clearing the slot we are about to install).
    invalidate_dm_chat_stream(session.as_ref(), &writers, peer);
    let generation = session.claim_dm_writer_generation(peer);
    let (tx, rx) = mpsc::unbounded_channel::<StreamWireItem>();
    if let Ok(mut g) = writers.lock() {
        g.insert(peer, tx);
    }
    session.set_dm_stream_writer(peer, true);
    *write_task = Some(spawn_dm_stream_write_task(
        peer,
        writer,
        rx,
        generation,
        Arc::clone(&writers),
        Arc::clone(&session),
    ));
    *owns_writer = true;
    *my_gen = generation;
    emit_chat_ready_if_can_send(
        Arc::clone(&session),
        peer,
        Arc::clone(&writers),
        events_tx,
    );
}

/// One long-lived `/ghal-bol/msg/1.0.0` per libp2p peer (read/write until reset).
async fn handle_inbound_stream(
    peer: PeerId,
    stream: libp2p::Stream,
    session: Arc<SessionState>,
    writers: StreamWriters,
    events_tx: Option<std::sync::mpsc::Sender<GossipChatEvent>>,
    _control: stream::Control,
) {
    if session.is_bootstrap_peer(peer) {
        return;
    }
    let (mut reader, writer) = stream.split();
    let mut spare_writer: Option<futures::io::WriteHalf<libp2p::Stream>> = None;
    // Writer generation owned by this handler. 0 = none (duplicate / read-only); set when this
    // handler installs or adopts the writer. Stale handlers compare this on teardown.
    let mut my_gen: u64 = 0;

    let mut owns_writer = {
        let mut owns = false;
        if let Ok(g) = writers.lock() {
            owns = !g.contains_key(&peer);
        }
        owns
    };
    let mut write_task = if owns_writer {
        let (tx, rx) = mpsc::unbounded_channel::<StreamWireItem>();
        let generation = session.claim_dm_writer_generation(peer);
        my_gen = generation;
        if let Ok(mut g) = writers.lock() {
            g.insert(peer, tx);
        }
        session.set_dm_stream_writer(peer, true);
        session.note_dm_inbound_activity(peer);
        session.ensure_dm_peer_from_libp2p(peer);
        emit_chat_ready_if_can_send(
            Arc::clone(&session),
            peer,
            Arc::clone(&writers),
            events_tx.clone(),
        );
        if let Some(pk) = session
            .dm_peer_for_libp2p(peer)
            .and_then(|d| d.public_key_hex.clone())
        {
            session.try_emit_peer_identified(peer, pk, &events_tx);
        }
        Some(spawn_dm_stream_write_task(
            peer,
            writer,
            rx,
            generation,
            Arc::clone(&writers),
            Arc::clone(&session),
        ))
    } else {
        spare_writer = Some(writer);
        None
    };

    // Symmetric connect: both sides may open `/ghal-bol/msg/1.0.0` before either accept wins the
    // writer slot. The peer's outbound stream arrives here as a "duplicate" — still process frames
    // (draining dropped guest→host text in coord/LAN tests when both sides opened outbound).
    if !owns_writer {
        native_log::debug(
            "stream",
            format!("inbound chat stream from {peer} — duplicate mux; read-only"),
        );
        // Phone re-dialed on relay while we still hold the writer on stale direct — adopt
        // immediately so their `open_stream` completes (TRANSPORT.md § duplicate-mux adoption).
        // **Only** `asymmetric_relay_recover_on_existing_link` here — not outbound-stuck/zombie at
        // stream-open (avoids churn on symmetric-connect duplicates; Symptom C).
        if asymmetric_relay_recover_on_existing_link(session.as_ref(), peer) {
            adopt_duplicate_mux_as_writer(
                peer,
                &mut spare_writer,
                &mut write_task,
                &mut owns_writer,
                &mut my_gen,
                Arc::clone(&session),
                Arc::clone(&writers),
                events_tx.clone(),
            );
        }
    }

    let identity = session.identity.clone();

    loop {
        let frame = match read_frame(&mut reader).await {
            Ok(f) => f,
            Err(e) => {
                if stream_read_is_terminal(&e) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
                continue;
            }
        };
        session.note_dm_inbound_activity(peer);
        let share = match frame_wire_share(&frame) {
            Ok(s) => s,
            Err(_) => continue,
        };
        if share == CALL_SHARE {
            let env = match call_envelope_from_frame(&frame) {
                Ok(e) => e,
                Err(_) => continue,
            };
            let peer_transport_pk = session.peer_transport_pk(env.sender_public_key_hex.trim());
            let parsed = match parse_call_envelope_with_transport(
                &env,
                &identity,
                peer_transport_pk.as_ref().map(|peer_pk| {
                    crate::call_sig_v1::CallOpenTransportCtx {
                        local_sk: session.dm_local_transport_sk(),
                        peer_pk,
                    }
                }),
            ) {
                Ok(p) => p,
                Err(e) => {
                    native_log::warn("call", format!("drop call frame from {peer}: {e}"));
                    continue;
                }
            };
            if !contact_identity_wire_matches_peer_id(&parsed.sender_public_key_hex, &peer) {
                native_log::warn(
                    "call",
                    format!("drop call from {peer}: signing key mismatch"),
                );
                continue;
            }
            if matches!(parsed.kind, crate::call_sig_v1::CallSigKind::Invite) {
                let now_ms = chrono_now_ms();
                if !call_invite_is_live(parsed.created_at_ms, now_ms) {
                    native_log::info(
                        "call",
                        format!(
                            "drop stale invite call_id={} from {peer} (age_ms={})",
                            parsed.call_id,
                            now_ms.saturating_sub(parsed.created_at_ms)
                        ),
                    );
                    drop_pending_call_invite(&parsed.call_id);
                    call_state::clear_peer(&parsed.sender_public_key_hex);
                    platform_incoming_call_dismiss();
                    continue;
                }
            }
            if let Err(e) = call_state::apply_inbound(
                &parsed.sender_public_key_hex,
                &parsed.call_id,
                parsed.kind,
            ) {
                native_log::debug(
                    "call",
                    format!(
                        "ignore inbound {} call_id={} from {peer}: {e}",
                        parsed.kind.wire_name(),
                        parsed.call_id
                    ),
                );
                continue;
            }
            match parsed.kind {
                crate::call_sig_v1::CallSigKind::Invite => {
                    let media_up = crate::p2p::call_active::snapshot().is_some();
                    let phase = call_state::peer_call_phase(&parsed.sender_public_key_hex);
                    // Ring only for a fresh inbound invite — never during live media or outbound ring.
                    if !media_up && phase == call_state::CallPhase::IncomingRinging {
                        platform_incoming_call_show(&parsed.sender_public_key_hex, &parsed.call_id);
                    }
                }
                crate::call_sig_v1::CallSigKind::Accept => {
                    platform_incoming_call_dismiss();
                }
                crate::call_sig_v1::CallSigKind::VideoOn => {
                    platform_incoming_call_dismiss();
                    crate::p2p::call_active::set_remote_video_on(&parsed.call_id, true);
                    emit_call_media(
                        &events_tx,
                        &parsed.call_id,
                        &parsed.sender_public_key_hex,
                        "remote_video_on",
                        None,
                    );
                }
                crate::call_sig_v1::CallSigKind::VideoOff => {
                    platform_incoming_call_dismiss();
                    crate::p2p::call_active::set_remote_video_on(&parsed.call_id, false);
                    emit_call_media(
                        &events_tx,
                        &parsed.call_id,
                        &parsed.sender_public_key_hex,
                        "remote_video_off",
                        None,
                    );
                }
                crate::call_sig_v1::CallSigKind::Hangup
                | crate::call_sig_v1::CallSigKind::Reject => {
                    platform_incoming_call_dismiss();
                    drop_pending_call_invite(&parsed.call_id);
                    let pk = parsed.sender_public_key_hex.clone();
                    let cid = parsed.call_id.clone();
                    if crate::p2p::call_active::snapshot().is_some_and(|s| s.call_id == cid) {
                        session.call_media_stop(&cid);
                        session.call_video_stop(&cid);
                        crate::p2p::call_active::clear();
                        emit_call_media(&events_tx, &cid, &pk, "call_ended", Some("remote_hangup"));
                    }
                }
                _ => {}
            }
            session.ensure_dm_peer(&parsed.sender_public_key_hex, peer);
            session.try_emit_peer_identified(
                peer,
                parsed.sender_public_key_hex.clone(),
                &events_tx,
            );
            if let Some(tx) = &events_tx {
                let _ = tx.send(GossipChatEvent::CallSignal {
                    from: peer,
                    id: parsed.id.clone(),
                    call_id: parsed.call_id.clone(),
                    signal: parsed.kind.wire_name().to_string(),
                    sender_public_key_hex: parsed.sender_public_key_hex.clone(),
                    created_at_ms: parsed.created_at_ms,
                    payload: parsed.payload.clone(),
                });
            }
            continue;
        }
        if share != MSG_SHARE {
            continue;
        }
        let env = match frame_bytes_to_envelope(&frame) {
            Ok(e) => e,
            Err(_) => continue,
        };
        let peer_transport_pk = session.peer_transport_pk(env.sender_public_key_hex.trim());
        let open_transport = peer_transport_pk.as_ref().map(|peer_pk| DmOpenTransportCtx {
            local_sk: session.dm_local_transport_sk(),
            peer_pk,
        });
        let parsed = match parse_envelope_with_transport(
            &env,
            &session.identity,
            open_transport,
        ) {
            Ok(p) => p,
            Err(e) => {
                native_log::warn("stream", format!("drop frame from {peer}: {e}"));
                continue;
            }
        };
        match parsed {
            ParsedMsg::TransportKemHello {
                sender_public_key_hex,
                transport_pk,
            } => {
                let had_pk = session
                    .peer_transport_pk(sender_public_key_hex.trim())
                    .is_some();
                session.store_peer_transport_pk(&sender_public_key_hex, transport_pk);
                native_log::debug(
                    "stream",
                    format!("transport kem hello from {peer} pk stored"),
                );
                if !had_pk && writer_open_for_peer(&writers, peer) {
                    emit_chat_ready_if_can_send(
                        Arc::clone(&session),
                        peer,
                        Arc::clone(&writers),
                        events_tx.clone(),
                    );
                }
                continue;
            }
            ParsedMsg::Text(t) => {
                let roster_wire = session
                    .dm_peer_for_libp2p(peer)
                    .and_then(|d| d.public_key_hex);
                if !sender_identity_matches_stream_peer(
                    &t.sender_public_key_hex,
                    &peer,
                    roster_wire.as_deref(),
                ) {
                    native_log::warn(
                        "stream",
                        format!("drop text from {peer}: signing key mismatch"),
                    );
                    continue;
                }
                // Flow milestone at info (App-log visible): a text message arrived on the wire.
                // One line per inbound id (duplicates log at debug below), so no spam.
                native_log::info(
                    "stream",
                    format!("inbound text from {peer} id={} len={}", t.id, t.text.len()),
                );
                let is_new = session.remember_inbound_id(&t.id, chrono_now_ms());
                let was_known = session
                    .dm_peer_for_libp2p(peer)
                    .is_some_and(|d| d.has_send_keys());
                session.ensure_dm_peer(&t.sender_public_key_hex, peer);
                if is_new && !was_known {
                    session.try_emit_peer_identified(
                        peer,
                        t.sender_public_key_hex.clone(),
                        &events_tx,
                    );
                    emit_chat_ready_if_can_send(
                        Arc::clone(&session),
                        peer,
                        Arc::clone(&writers),
                        events_tx.clone(),
                    );
                }
                let mut persisted_on_wire = false;
                if is_new {
                    let received_at_ms = session
                        .inbound_received_at_ms(&t.id)
                        .unwrap_or_else(chrono_now_ms);
                    persisted_on_wire = crate::dm_event_handler::persist_inbound_text_on_wire(
                        &peer.to_string(),
                        &t.id,
                        &t.text,
                        &t.sender_public_key_hex,
                        t.created_at_ms,
                        received_at_ms,
                    );
                    if !persisted_on_wire {
                        native_log::warn(
                            "DM/store",
                            format!(
                                "inbound text not persisted on wire id={} from {peer} (handler context?)",
                                t.id
                            ),
                        );
                    } else if let Some(tx) = &events_tx {
                        let _ = tx.send(GossipChatEvent::DmMessage {
                            from: peer,
                            id: t.id.clone(),
                            msg_kind: "text".to_string(),
                            text: Some(t.text.clone()),
                            ref_id: None,
                            sender_public_key_hex: t.sender_public_key_hex.clone(),
                            created_at_ms: t.created_at_ms,
                            received_at_ms: Some(received_at_ms),
                        });
                    }
                } else {
                    session.clear_delivery_ack_sent(&t.id);
                    native_log::debug(
                        "stream",
                        format!("duplicate text id={} from {peer} — ack retry only", t.id),
                    );
                }
                // A retransmitted inbound text (peer resending because it never got our ack) is
                // strong proof our owned writer is dead — take over so the ack actually reaches it.
                if !owns_writer
                    && duplicate_mux_should_take_over(
                        session.as_ref(),
                        peer,
                        chrono_now_ms(),
                        !is_new,
                    )
                {
                    adopt_duplicate_mux_as_writer(
                        peer,
                        &mut spare_writer,
                        &mut write_task,
                        &mut owns_writer,
                        &mut my_gen,
                        Arc::clone(&session),
                        Arc::clone(&writers),
                        events_tx.clone(),
                    );
                }
                // `:p2p` background must always send `ack_received` (UI may be dead; foreground
                // peer can be stale). In-room `ack_read` only after transcript persist succeeded.
                send_inbound_delivery_ack(
                    peer,
                    &t.id,
                    &t.sender_public_key_hex,
                    session.as_ref(),
                    &writers,
                )
                .await;
                let in_room = may_send_in_room_read_ack(session.as_ref(), peer)
                    && !session.is_read_ack_confirmed(&t.id);
                let may_read_ack = in_room && (!is_new || persisted_on_wire);
                if may_read_ack {
                    send_inbound_read_ack_if_possible(
                        peer,
                        &t.id,
                        &t.sender_public_key_hex,
                        session.as_ref(),
                        &writers,
                    )
                    .await;
                }
            }
            // Asymmetric ack routing (see `dm_delivery_sync.dart` / DESIGN.md):
            // - Off-room → `ack_received` only; in-room → `ack_received` then `ack_read`.
            // - Our outbound outbox clears on peer `ack_received` or `ack_read` for our message id.
            // - `ack_read` on our outbound id → peer read; `ack_received` on inbound id → read-receipt confirm.
            ParsedMsg::Ack(a) => {
                let roster_wire = session
                    .dm_peer_for_libp2p(peer)
                    .and_then(|d| d.public_key_hex);
                if !sender_identity_matches_stream_peer(
                    &a.sender_public_key_hex,
                    &peer,
                    roster_wire.as_deref(),
                ) {
                    native_log::warn(
                        "stream",
                        format!("drop ack from {peer}: signing key mismatch"),
                    );
                    continue;
                }
                // Peer acked us on this stream — if our owned writer is stuck, move to this one.
                if !owns_writer
                    && duplicate_mux_should_take_over(
                        session.as_ref(),
                        peer,
                        chrono_now_ms(),
                        false,
                    )
                {
                    adopt_duplicate_mux_as_writer(
                        peer,
                        &mut spare_writer,
                        &mut write_task,
                        &mut owns_writer,
                        &mut my_gen,
                        Arc::clone(&session),
                        Arc::clone(&writers),
                        events_tx.clone(),
                    );
                }
                if a.kind == MsgKind::AckRequest {
                    // Deprecated wire kind — we never send this. Recipient drives delivery via
                    // `ack_received` / `ack_read` only (see `docs/GHAL_BOL_DM_MSG_V1.md`).
                    native_log::debug(
                        "stream",
                        format!("ignore ack_request ref={} from {peer}", a.ref_id),
                    );
                    continue;
                }
                if a.kind == MsgKind::AckReceived {
                    // Peer confirms they got our `ack_read` for their text (ref_id = their message id).
                    // Only after we actually queued/sent `ack_read` — not merely because we saw inbound id.
                    // Must not treat as our outbound delivered (GHAL_BOL_DM_MSG_V1.md § ack_received).
                    if session.has_pending_read_ack(&a.ref_id) {
                        session.mark_read_ack_confirmed(&a.ref_id);
                        continue;
                    }
                    session.finalize_outbound_ack(&a.ref_id);
                }
                if a.kind == MsgKind::AckRead {
                    // Read implies delivery — stop outbox retry without a separate `ack_received`.
                    session.finalize_outbound_ack(&a.ref_id);
                    session.ensure_dm_peer(&a.sender_public_key_hex, peer);
                    let _ = send_ack_frame(
                        peer,
                        &a.sender_public_key_hex,
                        &a.ref_id,
                        MsgKind::AckReceived,
                        session.as_ref(),
                        &writers,
                        None,
                    )
                    .await;
                }
                let kind = match a.kind {
                    MsgKind::AckReceived => "ack_received",
                    MsgKind::AckRead => "ack_read",
                    MsgKind::Text | MsgKind::AckRequest | MsgKind::TransportKemHello => continue,
                };
                // Flow milestone at info (App-log visible): the peer confirmed our message — this is
                // the sender-side tick transition (`ack_received` → delivered, `ack_read` → read).
                // Normal volume is ~1 delivered + 1 read per message (read-ack floods are a bug, not
                // normal — see DESIGN.md § "Read receipts"), so info here keeps the flow truthful.
                native_log::info(
                    "stream",
                    format!(
                        "{kind} from {peer} ref={} — our outbound {}",
                        a.ref_id,
                        if a.kind == MsgKind::AckRead { "read by peer" } else { "delivered" },
                    ),
                );
                if let Some(tx) = &events_tx {
                    let _ = tx.send(GossipChatEvent::DmMessage {
                        from: peer,
                        id: a.id.clone(),
                        msg_kind: kind.to_string(),
                        text: None,
                        ref_id: Some(a.ref_id.clone()),
                        sender_public_key_hex: a.sender_public_key_hex.clone(),
                        created_at_ms: a.created_at_ms,
                        received_at_ms: a.received_at_ms,
                    });
                }
            }
        }
    }

    if owns_writer {
        // Only tear down if this handler's generation is still the live writer — a newer mux
        // (adopt / reopen) may have taken over, and clearing here would kill the working writer.
        finalize_dm_writer_if_current(session.as_ref(), &writers, peer, my_gen);
        if let Some(task) = write_task.take() {
            let _ = task.await;
        }
    }
}

