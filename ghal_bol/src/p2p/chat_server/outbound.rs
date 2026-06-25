/// Room open/close and peer register must not sit behind a long `send_text_dm` queue.
fn outbound_cmd_priority(cmd: &OutboundCmd) -> u8 {
    match cmd {
        OutboundCmd::SetForegroundPeer { .. } | OutboundCmd::RunReadAckCatchup { .. } => 0,
        // Call media control-plane is latency sensitive — never queue behind text.
        OutboundCmd::CallMediaStart { .. }
        | OutboundCmd::CallMediaStop { .. }
        | OutboundCmd::CallMediaSetMicMuted { .. }
        | OutboundCmd::CallMediaSetSpeaker { .. }
        | OutboundCmd::CallVideoStart { .. }
        | OutboundCmd::CallVideoStop { .. }
        | OutboundCmd::CallVideoSetCameraEnabled { .. } => 0,
        OutboundCmd::RegisterDmPeer { .. } => 1,
        OutboundCmd::DialBootstrapPeers { .. } => 2,
        OutboundCmd::SendAck { .. } => 3,
        OutboundCmd::SendCallSignal { .. } => 4,
        OutboundCmd::SendText { .. } => 5,
    }
}

async fn drain_outbound_queue(
    outbound_rx: &std::sync::mpsc::Receiver<OutboundCmd>,
    swarm: &mut Swarm<ChatBehaviour>,
    session: Arc<SessionState>,
    writers: StreamWriters,
    control: stream::Control,
    events_tx: Option<std::sync::mpsc::Sender<GossipChatEvent>>,
    max_cmds: usize,
) {
    let mut batch = Vec::with_capacity(max_cmds.min(64));
    for _ in 0..max_cmds {
        let Ok(cmd) = outbound_rx.try_recv() else {
            break;
        };
        batch.push(cmd);
    }
    if batch.is_empty() {
        return;
    }
    batch.sort_by_key(|c| outbound_cmd_priority(c));
    for cmd in batch {
        let send_msg_id = match &cmd {
            OutboundCmd::SendText { message_id, .. } => Some(message_id.clone()),
            _ => None,
        };
        if let Err(e) = process_outbound_cmd(
            cmd,
            swarm,
            Arc::clone(&session),
            Arc::clone(&writers),
            control.clone(),
            events_tx.clone(),
        )
        .await
        {
            if let Some(tx) = &events_tx {
                if let Some(message_id) = send_msg_id {
                    // Outbox resends until `ack_received`; do not surface terminal send_failed.
                    if !session.outbox_contains(&message_id) && !is_transient_outbound_error(&e) {
                        let _ = tx.send(GossipChatEvent::SendFailed {
                            message_id,
                            error: e,
                        });
                    }
                } else {
                    let _ = tx.send(GossipChatEvent::DialFailed {
                        peer: None,
                        error: e,
                    });
                }
            }
        }
    }
}

async fn process_outbound_cmd(
    cmd: OutboundCmd,
    swarm: &mut Swarm<ChatBehaviour>,
    session: Arc<SessionState>,
    writers: StreamWriters,
    control: stream::Control,
    events_tx: Option<std::sync::mpsc::Sender<GossipChatEvent>>,
) -> Result<(), String> {
    if let OutboundCmd::RegisterDmPeer {
        peer_id,
        public_key_hex,
    } = &cmd
    {
        session.register_dm_peer_key(*peer_id, public_key_hex);
        if let (Some(path), Some(ns)) = (&session.transcript_path, &session.app_namespace) {
            transcript_sync_outbound_tick(session.as_ref(), Path::new(path), ns.trim());
        }
        let pk = public_key_hex.trim();
        if pk.len() == 66 {
            if let Ok(derived) = peer_id_from_secp256k1_public_key_hex(pk) {
                if let Ok(target) = derived.parse::<PeerId>() {
                    if !dm_peer_chat_link_stable(
                        swarm,
                        session.as_ref(),
                        target,
                        Some(pk),
                        chrono_now_ms(),
                    ) {
                        notify_coord_lookup();
                    }
                }
            }
        } else if let Some(pid) = *peer_id {
            let now = chrono_now_ms();
            if !dm_peer_chat_link_stable(swarm, session.as_ref(), pid, None, now) {
                notify_coord_lookup();
            }
        }
        return Ok(());
    }
    if let OutboundCmd::DialBootstrapPeers { addrs } = &cmd {
        for ma in addrs {
            if crate::p2p::network_transport::is_trusted_bootstrap_dial_addr(ma) {
                let _ = swarm.dial(ma.clone());
            }
        }
        return Ok(());
    }
    if let OutboundCmd::CallMediaStart {
        call_id,
        peer_public_key_hex,
    } = &cmd
    {
        let pk = peer_public_key_hex.trim().to_string();
        if pk.len() == 66 {
            session.register_dm_peer_key(None, &pk);
            // Signaling may have connected already; upkeep owns connect — wake coord only.
            if let Some(peer) = session.resolve_send_peer(&pk) {
                if !swarm.is_connected(&peer) {
                    notify_coord_lookup();
                }
            }
        }
        let session2 = Arc::clone(&session);
        let control2 = control.clone();
        let call_id = call_id.clone();
        let events_tx2 = events_tx.clone();
        tokio::spawn(async move {
            if let Err(e) =
                start_call_media(session2, control2, call_id.clone(), pk, events_tx2).await
            {
                native_log::warn("call_media", format!("start failed call_id={call_id}: {e}"));
            }
        });
        return Ok(());
    }
    if let OutboundCmd::CallMediaStop { call_id } = &cmd {
        let pk = crate::p2p::call_active::snapshot()
            .filter(|s| s.call_id == *call_id)
            .map(|s| s.peer_public_key_hex.clone());
        let stopped = session.call_media_stop(call_id);
        crate::p2p::call_active::on_voice_stop(call_id);
        if let Some(pk) = pk {
            emit_call_media(&events_tx, call_id, &pk, "voice_stopped", None);
            if crate::p2p::call_active::snapshot().is_none() {
                call_state::clear_peer(&pk);
                platform_incoming_call_dismiss();
                emit_call_media(
                    &events_tx,
                    call_id,
                    &pk,
                    "call_ended",
                    Some("media_stopped"),
                );
            }
        }
        #[cfg(target_os = "android")]
        crate::call_media::reset_voice_audio_mode_flag();
        native_log::info(
            "call_media",
            format!("stop call_id={call_id} (was_active={stopped})"),
        );
        return Ok(());
    }
    if let OutboundCmd::CallVideoStart {
        call_id,
        peer_public_key_hex,
        camera_enabled,
    } = &cmd
    {
        let pk = peer_public_key_hex.trim().to_string();
        if pk.len() == 66 {
            session.register_dm_peer_key(None, &pk);
            if let Some(peer) = session.resolve_send_peer(&pk) {
                if !swarm.is_connected(&peer) {
                    notify_coord_lookup();
                }
            }
        }
        // Await setup so follow-up `set_camera_enabled` never races an unregistered session.
        if let Err(e) = start_call_video(
            Arc::clone(&session),
            control.clone(),
            call_id.clone(),
            pk.clone(),
            *camera_enabled,
        )
        .await
        {
            native_log::warn("call_video", format!("start failed call_id={call_id}: {e}"));
            return Err(e);
        }
        emit_call_media(&events_tx, call_id, &pk, "video_started", None);
        return Ok(());
    }
    if let OutboundCmd::CallVideoStop { call_id } = &cmd {
        let pk = crate::p2p::call_active::snapshot()
            .filter(|s| s.call_id == *call_id)
            .map(|s| s.peer_public_key_hex.clone());
        let stopped = session.call_video_stop(call_id);
        crate::p2p::call_active::on_video_stop(call_id);
        if let Some(pk) = pk {
            emit_call_media(&events_tx, call_id, &pk, "video_stopped", None);
            if crate::p2p::call_active::snapshot().is_none() {
                call_state::clear_peer(&pk);
                platform_incoming_call_dismiss();
                emit_call_media(
                    &events_tx,
                    call_id,
                    &pk,
                    "call_ended",
                    Some("video_stopped"),
                );
            }
        }
        native_log::info(
            "call_video",
            format!("stop call_id={call_id} (was_active={stopped})"),
        );
        return Ok(());
    }
    if let OutboundCmd::CallVideoSetCameraEnabled { call_id, enabled } = &cmd {
        let mut ok = session.call_video_set_camera_off(call_id, !*enabled);
        if !ok {
            let session2 = Arc::clone(&session);
            let call_id2 = call_id.clone();
            let enabled2 = *enabled;
            for attempt in 1..=25 {
                tokio::time::sleep(Duration::from_millis(40)).await;
                ok = session2.call_video_set_camera_off(&call_id2, !enabled2);
                if ok {
                    native_log::info(
                        "call_video",
                        format!(
                            "set_camera_enabled call_id={call_id2} enabled={enabled2} applied=true (retry {attempt})",
                        ),
                    );
                    break;
                }
            }
        }
        if ok {
            crate::p2p::call_active::set_camera_on(call_id, *enabled);
        }
        native_log::info(
            "call_video",
            format!("set_camera_enabled call_id={call_id} enabled={enabled} applied={ok}"),
        );
        return Ok(());
    }
    if let OutboundCmd::CallMediaSetMicMuted { call_id, muted } = &cmd {
        let ok = session.call_media_set_mic_muted(call_id, *muted);
        native_log::debug(
            "call_media",
            format!("set_mic_muted call_id={call_id} muted={muted} applied={ok}"),
        );
        return Ok(());
    }
    if let OutboundCmd::CallMediaSetSpeaker {
        call_id,
        speaker_on,
    } = &cmd
    {
        match crate::call_media::set_speakerphone(*speaker_on) {
            Ok(()) => native_log::info(
                "call_media",
                format!(
                    "set_speaker call_id={call_id} speaker_on={speaker_on} active={}",
                    session.call_media_active(call_id)
                ),
            ),
            Err(e) => native_log::warn(
                "call_media",
                format!("set_speaker call_id={call_id} failed: {e}"),
            ),
        }
        return Ok(());
    }
    if let OutboundCmd::RunReadAckCatchup { peer_id } = &cmd {
        let peer = *peer_id;
        // Always seed from transcript first — throttle only the burst drain (hub nudge / resume).
        seed_read_acks_for_peer_from_transcript(session.as_ref(), peer);
        if !may_send_in_room_read_ack(session.as_ref(), peer) {
            native_log::debug(
                "read_ack",
                format!("catch-up {peer} deferred — read gate off; transcript backlog seeded"),
            );
            return Ok(());
        }
        if read_ack_catchup_throttled(peer, chrono_now_ms()) {
            return Ok(());
        }
        native_log::info(
            "read_ack",
            format!("read gate opened — catch-up ack_read for foreground {peer}"),
        );
        let session2 = Arc::clone(&session);
        let writers2 = Arc::clone(&writers);
        let control2 = control.clone();
        tokio::spawn(async move {
            read_ack_catchup_for_peer(session2, writers2, peer, true, false, Some(control2)).await;
        });
        return Ok(());
    }
    if let OutboundCmd::SetForegroundPeer {
        peer_id,
        generation,
    } = &cmd
    {
        if *generation < foreground_peer_cmd_gen_latest() {
            native_log::debug(
                "read_ack",
                format!(
                    "skip stale SetForegroundPeer gen={generation} latest={}",
                    foreground_peer_cmd_gen_latest()
                ),
            );
            return Ok(());
        }
        let previous = session.current_foreground_peer().or_else(|| {
            last_room_peer().and_then(|pk| {
                peer_id_from_secp256k1_public_key_hex(&pk)
                    .ok()
                    .and_then(|s| s.parse().ok())
            })
        });
        session.set_foreground_peer(*peer_id);
        if let Some(left) = previous {
            let leaving = match peer_id {
                None => true,
                Some(new) => *new != left,
            };
            if leaving {
                spawn_leave_read_ack_drain(
                    Arc::clone(&session),
                    Arc::clone(&writers),
                    left,
                    control.clone(),
                );
            }
        }
        if peer_id.is_none() {
            if let Ok(mut last) = last_room_peer_mx().write() {
                *last = None;
            }
            return Ok(());
        }
        let peer = peer_id.unwrap();
        if session.current_foreground_peer() != Some(peer) {
            native_log::debug(
                "read_ack",
                format!("chat room enter {peer} skipped — foreground already changed"),
            );
            return Ok(());
        }
        seed_read_acks_for_peer_from_transcript(session.as_ref(), peer);
        if !app_ack_read_enabled() || !app_ui_visible() {
            native_log::debug(
                "read_ack",
                format!(
                    "chat room enter {peer} deferred — read gate off; seeded transcript backlog"
                ),
            );
            return Ok(());
        }
        native_log::info(
            "read_ack",
            format!("chat room enter {peer} — ack_read for in-room backlog only"),
        );
        if let (Some(path), Some(ns)) = (&session.transcript_path, &session.app_namespace) {
            transcript_sync_outbound_tick(session.as_ref(), Path::new(path), ns.trim());
        }
        let session2 = Arc::clone(&session);
        let writers2 = Arc::clone(&writers);
        let control2 = control.clone();
        tokio::spawn(async move {
            if session2.current_foreground_peer() != Some(peer) {
                return;
            }
            if !may_send_in_room_read_ack(session2.as_ref(), peer) {
                return;
            }
            read_ack_catchup_for_peer(session2, writers2, peer, true, false, Some(control2)).await;
        });
        return Ok(());
    }

    let mut sent_text_id: Option<String> = None;
    let mut pending_call: Option<PendingCallSignal> = None;
    let (peer, frame, done) = match cmd {
        OutboundCmd::RegisterDmPeer { .. }
        | OutboundCmd::SetForegroundPeer { .. }
        | OutboundCmd::RunReadAckCatchup { .. }
        | OutboundCmd::DialBootstrapPeers { .. }
        | OutboundCmd::CallMediaStart { .. }
        | OutboundCmd::CallMediaStop { .. }
        | OutboundCmd::CallMediaSetMicMuted { .. }
        | OutboundCmd::CallMediaSetSpeaker { .. }
        | OutboundCmd::CallVideoStart { .. }
        | OutboundCmd::CallVideoStop { .. }
        | OutboundCmd::CallVideoSetCameraEnabled { .. } => unreachable!(),
        OutboundCmd::SendCallSignal {
            recipient_public_key_hex,
            call_id,
            signal_kind,
            payload,
            signal_id,
        } => {
            let pk = recipient_public_key_hex.trim();
            if pk.len() == 66 {
                session.register_dm_peer_key(None, pk);
            }
            let peer = session
                .resolve_send_peer(pk)
                .ok_or_else(|| "unknown contact — add them via invitation first".to_string())?;
            session.ensure_dm_peer_from_libp2p(peer);
            if let Err(e) = call_state::apply_outbound(pk, &call_id, signal_kind) {
                return Err(e);
            }
            if matches!(signal_kind, CallSigKind::Hangup | CallSigKind::Reject) {
                session.purge_pending_call_signals(&call_id);
            }
            on_local_call_signal_sent(&call_id, signal_kind);
            let env = build_call_envelope(
                &signal_id,
                &call_id,
                signal_kind,
                session.identity.keypair(),
                pk,
                payload,
                chrono_now_ms(),
            )?;
            let frame = call_envelope_to_frame_bytes(&env)?;
            native_log::info(
                "call",
                format!(
                    "enqueue call signal {} call_id={} peer={peer}",
                    signal_kind.wire_name(),
                    call_id
                ),
            );
            pending_call = Some(PendingCallSignal {
                call_id: call_id.clone(),
                signal_kind,
                frame,
                peer_id: peer,
                recipient_public_key_hex: pk.to_string(),
                created_at_ms: chrono_now_ms(),
            });
            (peer, Vec::new(), None)
        }
        OutboundCmd::SendText {
            recipient_public_key_hex,
            text,
            message_id,
            done,
        } => {
            sent_text_id = Some(message_id.clone());
            let pk = recipient_public_key_hex.trim();
            if pk.len() == 66 {
                session.register_dm_peer_key(None, pk);
            }
            let peer = session.resolve_send_peer(pk).ok_or_else(|| {
                let err = "unknown contact — add them via invitation first".to_string();
                native_log::warn(
                    "outbound",
                    format!("send_text rejected: {err} pk={pk} msg_id={message_id}"),
                );
                err
            })?;
            session.ensure_dm_peer_from_libp2p(peer);
            // Stream-first: upkeep owns connect; wake coord lookup only.
            if !swarm.is_connected(&peer) {
                // Intent beats backoff (TRANSPORT.md § prime directive): an explicit send opens the
                // urgent window even for a peer last seen offline, so the next lookup pass finds
                // them within seconds if they are reachable now. The 30s window bounds retries for a
                // genuinely-offline contact, and `mark_dm_reconnect_urgent` no-ops if already armed.
                if pk.len() == 66 {
                    session.mark_dm_reconnect_urgent(pk);
                }
                notify_coord_lookup();
            }
            let remote = session
                .dm_peer(pk)
                .ok_or_else(|| "unknown dm peer — add contact first".to_string())?;
            let _recipient_pk = remote
                .public_key_hex
                .as_deref()
                .ok_or_else(|| "peer public key not known yet — wait for connect".to_string())?;
            let now = chrono_now_ms();
            let pending = PendingOutbound {
                message_id: message_id.clone(),
                peer_id: peer,
                recipient_public_key_hex: recipient_public_key_hex.clone(),
                text: text.clone(),
                created_at_ms: now,
                last_send_ms: now,
                on_wire: false,
            };
            let frame_bytes = build_pending_outbound_frame(session.as_ref(), &pending)?;
            session.track_outbound(pending);
            if let Some(ns) = session
                .app_namespace
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                let conv_key = recipient_public_key_hex.trim().to_string();
                if conv_key.len() == 66 {
                    let line = crate::dm_transcript_store::StoredChatLine {
                        local_id: message_id.clone(),
                        text: text.clone(),
                        outgoing: true,
                        from: None,
                        message_id: Some(message_id.clone()),
                        delivery: "pending".to_string(),
                        created_at_ms: Some(now),
                        read_ack_sent: false,
                    };
                    match crate::dm_transcript_store::append_if_new(ns, &conv_key, line) {
                        Ok(()) => native_log::info(
                            "outbound",
                            format!(
                                "transcript append outbound msg_id={message_id} conv={conv_key}"
                            ),
                        ),
                        Err(e) => native_log::warn(
                            "outbound",
                            format!("transcript append outbound failed msg_id={message_id}: {e}"),
                        ),
                    }
                }
            }
            native_log::info(
                "outbound",
                format!(
                    "enqueue send_text peer={peer} msg_id={message_id} text_len={}",
                    text.len()
                ),
            );
            (peer, frame_bytes, done)
        }
        OutboundCmd::SendAck {
            recipient_public_key_hex,
            ref_id,
            ack_kind,
        } => {
            let peer_id = session
                .resolve_send_peer(&recipient_public_key_hex)
                .ok_or_else(|| "unknown dm peer".to_string())?;
            let env = build_ack_envelope(
                &new_msg_id(),
                &ref_id,
                ack_kind,
                session.identity.keypair(),
                &recipient_public_key_hex,
                chrono_now_ms(),
            )?;
            (peer_id, envelope_to_frame_bytes(&env)?, None)
        }
    };
    if let Some(call) = pending_call {
        if !swarm.is_connected(&peer) {
            native_log::info(
                "dial",
                format!("call signal queued: connect {peer} (not connected yet)"),
            );
            notify_coord_lookup();
            session.enqueue_pending_call_signal(call);
            return Ok(());
        }
        ensure_dm_stream_for_send(
            peer,
            Arc::clone(&session),
            control.clone(),
            Arc::clone(&writers),
            events_tx.clone(),
        )
        .await;
        if !writer_open_for_peer(&writers, peer) {
            native_log::info(
                "call",
                format!("call signal queued: stream opening (peer={peer})"),
            );
            session.enqueue_pending_call_signal(call);
            return Ok(());
        }
        let r = send_frame_to_peer(peer, call.frame, writers, Some(session.as_ref())).await;
        if r.is_ok() {
            native_log::info(
                "call",
                format!(
                    "call frame on wire peer={peer} {} call_id={}",
                    call.signal_kind.wire_name(),
                    call.call_id
                ),
            );
            if let Some(tx) = events_tx.as_ref() {
                let _ = tx.send(GossipChatEvent::CallSignalSent {
                    call_id: call.call_id.clone(),
                    signal: call.signal_kind.wire_name().to_string(),
                    recipient_public_key_hex: call.recipient_public_key_hex.clone(),
                });
            }
        }
        return r;
    }
    if !swarm.is_connected(&peer) {
        if let Some(pk) = session
            .dm_peer_for_libp2p(peer)
            .and_then(|d| d.public_key_hex.clone())
        {
            session.mark_dm_reconnect_urgent(&pk);
        }
        connect_dm_peer_now(swarm, session.as_ref(), peer);
        native_log::info(
            "dial",
            format!("outbound waiting: not connected to {peer} — connect in progress"),
        );
        let err = "connecting to peer — try send again in a moment".to_string();
        if let Some(done) = done {
            let _ = done.send(Err(err.clone()));
        }
        return Err(err);
    }
    ensure_dm_stream_for_send(
        peer,
        Arc::clone(&session),
        control.clone(),
        Arc::clone(&writers),
        events_tx.clone(),
    )
    .await;
    if !writer_open_for_peer(&writers, peer) {
        let err = format!("open_stream writer wait timed out for {peer}");
        native_log::info("stream", format!("{err} (peer={peer})"));
        if let Some(done) = done {
            let _ = done.send(Err(err.clone()));
        }
        return Err(err);
    }
    let r = send_frame_to_peer(peer, frame, Arc::clone(&writers), Some(session.as_ref())).await;
    if r.is_ok() {
        if let Some(id) = sent_text_id {
            notify_outbound_on_wire(&session, &id, chrono_now_ms(), &events_tx);
            native_log::info("outbound", format!("frame on wire peer={peer} msg_id={id}"));
        }
    } else if let (Some(id), Err(e)) = (&sent_text_id, &r) {
        session.mark_outbox_send_failed(id, chrono_now_ms());
        native_log::warn(
            "outbound",
            format!("send_frame failed peer={peer} msg_id={id}: {e} — outbox will retry"),
        );
        let err_s = e.to_string();
        if err_s.contains("chat stream closed") || err_s.contains("no chat stream") {
            if let Ok(mut g) = writers.lock() {
                g.remove(&peer);
            }
            session.request_dm_stream_reopen(peer);
        }
    }
    if let Some(done) = done {
        let _ = done.send(r.clone());
    }
    r
}

fn pair_from_dm(session: &SessionState, dm: &DmPeer) -> Option<(PeerId, String)> {
    let pk = dm.public_key_hex.as_deref()?.trim().to_string();
    if pk.len() != 66 {
        return None;
    }
    let peer_id = session.resolve_send_peer(&pk)?;
    Some((peer_id, pk))
}

fn resolve_send_pair_for_row(
    session: &SessionState,
    row: &crate::dm_transcript_v1::PendingOutboundRow,
) -> Option<(PeerId, String)> {
    if let Some(dm) = session.dm_peer_for_conversation_key(&row.conversation_key) {
        return pair_from_dm(session, &dm);
    }
    let ck = row.conversation_key.trim();
    for peer_id in session.dm_peer_ids() {
        let Some(dm) = session.dm_peer_for_libp2p(peer_id) else {
            continue;
        };
        let pk = dm.public_key_hex.as_deref().unwrap_or("").trim();
        if ck == peer_id.to_string() || (pk.len() == 66 && ck == pk) {
            return pair_from_dm(session, &dm);
        }
    }
    let ids = session.dm_peer_ids();
    if ids.len() == 1 {
        return session
            .dm_peer_for_libp2p(ids[0])
            .and_then(|dm| pair_from_dm(session, &dm));
    }
    None
}

fn build_pending_outbound_frame(
    session: &SessionState,
    p: &PendingOutbound,
) -> Result<Vec<u8>, String> {
    let remote = session
        .dm_peer(&p.recipient_public_key_hex)
        .ok_or_else(|| "unknown dm peer".to_string())?;
    let recipient_pk = remote
        .public_key_hex
        .as_deref()
        .ok_or_else(|| "peer public key missing".to_string())?;
    let ts = if p.created_at_ms > 0 {
        p.created_at_ms
    } else {
        chrono_now_ms()
    };
    let env = build_text_envelope(
        &p.message_id,
        session.identity.keypair(),
        recipient_pk,
        &p.text,
        ts,
    )?;
    envelope_to_frame_bytes(&env)
}

fn merge_outbound_row_into_outbox(
    session: &SessionState,
    row: &crate::dm_transcript_v1::PendingOutboundRow,
) -> bool {
    if session.outbox_contains(&row.message_id) {
        return false;
    }
    let Some((peer_id, pk)) = resolve_send_pair_for_row(session, row) else {
        return false;
    };
    let now = chrono_now_ms();
    session.track_outbound(PendingOutbound {
        message_id: row.message_id.clone(),
        peer_id,
        recipient_public_key_hex: pk,
        text: row.text.clone(),
        created_at_ms: if row.created_at_ms > 0 {
            row.created_at_ms
        } else {
            now
        },
        last_send_ms: now,
        on_wire: false,
    });
    true
}

