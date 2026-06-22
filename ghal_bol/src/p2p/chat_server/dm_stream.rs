fn stream_open_needs_connection_reset(err: &str) -> bool {
    let e = err.to_lowercase();
    e.contains("receiver is gone") || e.contains("oneshot canceled")
}

fn is_direct_lan_tcp_ma(ma: &Multiaddr) -> bool {
    if !is_tcp_multiaddr(ma) || crate::p2p::network_transport::is_relay_circuit_multiaddr(ma) {
        return false;
    }
    crate::p2p::network_transport::ipv4_from_ma_str(&ma.to_string()).is_some_and(|ip| ip.is_private())
}

/// Peer may be reachable on local mDNS/LAN (not a mobile-data / off-LAN WAN-only contact).
fn peer_expects_lan_discovery(session: &SessionState, peer: PeerId) -> bool {
    if session.peer_on_local_lan(peer)
        || session.lan_listen_rediscovery_requested(peer)
        || session.peer_mdns_lan_addr(peer).is_some()
    {
        return true;
    }
    // First connect on Wi‑Fi: active-intent peers must get LAN discovery even without prior LAN history.
    session.network_profile_snapshot().has_active_lan()
        && (session.is_foreground_peer(peer)
            || session.is_peer_reconnect_urgent(peer, chrono_now_ms())
            || session.peer_has_pending_outbox(peer))
}

/// TRANSPORT.md § Hybrid coord presence — gate full LAN kicks (not all roster peers on Wi‑Fi).
fn peer_eligible_for_lan_handover(session: &SessionState, peer: PeerId) -> bool {
    if peer_expects_lan_discovery(session, peer) {
        return true;
    }
    // Background contact with queued outbox on Wi‑Fi — only when peer might be local.
    session.network_profile_snapshot().has_active_lan()
        && session.peer_has_pending_outbox(peer)
        && (session.peer_on_local_lan(peer) || session.lan_listen_rediscovery_requested(peer))
}

fn lan_rediscovery_peer_set(session: &SessionState, lan_history: Vec<PeerId>) -> Vec<PeerId> {
    if !lan_history.is_empty() {
        return lan_history;
    }
    session
        .dm_peer_ids()
        .into_iter()
        .filter(|p| peer_eligible_for_lan_handover(session, *p))
        .collect()
}

fn is_direct_lan_tcp_mdns_candidate(ma: &Multiaddr) -> bool {
    !is_quic_multiaddr(ma)
        && is_tcp_multiaddr(ma)
        && !crate::p2p::network_transport::is_relay_circuit_multiaddr(ma)
        && crate::p2p::network_transport::ipv4_from_ma_str(&ma.to_string())
            .is_some_and(|ip| ip.is_private())
        && crate::p2p::network_transport::is_dm_dial_multiaddr(ma)
}

fn failed_dial_multiaddr_from_error(err_s: &str) -> Option<Multiaddr> {
    let needle = err_s.find("/ip4/").or_else(|| err_s.find("/ip6/"))?;
    let rest = &err_s[needle..];
    let end = rest
        .find(": :")
        .or_else(|| rest.find("): "))
        .unwrap_or(rest.len());
    let candidate = rest[..end].trim_end_matches(')').trim();
    candidate.parse().ok()
}

async fn ensure_dm_stream_for_send(
    peer: PeerId,
    session: Arc<SessionState>,
    control: stream::Control,
    writers: StreamWriters,
    events_tx: Option<std::sync::mpsc::Sender<GossipChatEvent>>,
) {
    if writer_open_for_peer(&writers, peer) {
        return;
    }
    if !session.try_begin_stream_open(peer) {
        return;
    }
    let open_err = ensure_chat_stream(peer, control, writers, Arc::clone(&session), events_tx)
        .await
        .err();
    session.end_stream_open(peer);
    if let Some(e) = open_err {
        session.note_stream_open_failure(peer, &e);
    }
}

/// Open `/ghal-bol/msg/1.0.0` if missing (persistent stream).
async fn ensure_chat_stream(
    peer: PeerId,
    mut control: stream::Control,
    writers: StreamWriters,
    session: Arc<SessionState>,
    events_tx: Option<std::sync::mpsc::Sender<GossipChatEvent>>,
) -> Result<(), String> {
    // libp2p-stream auto-dials peer_id-only (all peerstore addrs) when not connected — polluted
    // identify/docker/quic addrs cause WAN failures on mobile (TRANSPORT.md § WAN coord dials).
    if !session.libp2p_peer_connected(peer) {
        return Err(format!("open_stream {peer}: not connected"));
    }
    if writer_open_for_peer(&writers, peer) {
        emit_chat_ready_if_can_send(
            Arc::clone(&session),
            peer,
            Arc::clone(&writers),
            events_tx.clone(),
        );
        return Ok(());
    }
    if session.log_stream_open_once(peer) {
        native_log::debug("stream", format!("open outbound chat stream to {peer}"));
    }
    let mut last_err = String::new();
    let mut stream = None;
    for attempt in 0..3u8 {
        match control
            .open_stream(peer, StreamProtocol::new(STREAM_PROTOCOL))
            .await
        {
            Ok(s) => {
                stream = Some(s);
                break;
            }
            Err(e) => {
                last_err = format!("open_stream {peer}: {e}");
                if stream_open_needs_connection_reset(&last_err) {
                    return Err(last_err);
                }
                let transient = last_err.contains("Connection refused");
                if attempt < 2 && transient {
                    tokio::time::sleep(Duration::from_millis(40 * (attempt as u64 + 1))).await;
                    continue;
                }
                return Err(last_err);
            }
        }
    }
    let stream = stream.ok_or_else(|| last_err)?;
    tokio::spawn(handle_inbound_stream(
        peer,
        stream,
        Arc::clone(&session),
        writers.clone(),
        events_tx.clone(),
        control,
    ));
    // Wait for handle_inbound_stream to install the mux writer (Android relay dials need >24 yields).
    let deadline = time::Instant::now() + Duration::from_secs(4);
    while time::Instant::now() < deadline {
        if writer_open_for_peer(&writers, peer) {
            emit_chat_ready_if_can_send(
                Arc::clone(&session),
                peer,
                Arc::clone(&writers),
                events_tx.clone(),
            );
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    Err("chat stream opening — try send again shortly".to_string())
}

async fn open_outbound_stream_if_needed(
    peer: PeerId,
    control: stream::Control,
    writers: StreamWriters,
    session: Arc<SessionState>,
    events_tx: Option<std::sync::mpsc::Sender<GossipChatEvent>>,
) {
    if writer_open_for_peer(&writers, peer) {
        return;
    }
    if !session.libp2p_peer_connected(peer) {
        return;
    }
    if should_defer_stream_open_for_wan_mux(session.as_ref(), peer) {
        return;
    }
    if !session.try_begin_stream_open(peer) {
        // Another task is already trying; avoid creating a self-cancel storm.
        return;
    }
    if let Err(e) = ensure_chat_stream(
        peer,
        control,
        writers,
        Arc::clone(&session),
        events_tx.clone(),
    )
    .await
    {
        session.note_stream_open_failure(peer, &e);
        if let Some(tx) = events_tx {
            if session.should_emit_stream_open_dial_failed(peer, chrono_now_ms()) {
                let _ = tx.send(GossipChatEvent::DialFailed {
                    peer: Some(peer),
                    error: format!("open chat stream: {e}"),
                });
            }
        }
    }
    // Always release, success or fail.
    session.end_stream_open(peer);
}

async fn on_dm_peer_connected(
    session: Arc<SessionState>,
    control: stream::Control,
    writers: StreamWriters,
    connected: PeerId,
    events_tx: Option<std::sync::mpsc::Sender<GossipChatEvent>>,
) {
    if !session.is_dm_contact(connected) {
        return;
    }
    session.ensure_dm_peer_from_libp2p(connected);
    if let Some(pk) = session
        .dm_peer_for_libp2p(connected)
        .and_then(|d| d.public_key_hex.clone())
    {
        session.try_emit_peer_identified(connected, pk, &events_tx);
    }
    open_outbound_stream_if_needed(connected, control, writers, Arc::clone(&session), events_tx)
        .await;
}

/// Native voice-call media substream. One per direction per call: each side
/// **opens** one (its TX) and **accepts** one (its RX). First frame is a small
/// plaintext header `{"call_id":"…"}`; all later frames are sealed media packets.
/// See `docs/GHAL_BOL_CALL_NATIVE_V2.md`.
pub(crate) const CALL_STREAM_PROTOCOL: &str = "/ghal-bol/call/1.0.0";

/// Native **video**-call media substream — same framing as [`CALL_STREAM_PROTOCOL`]
/// (plaintext `{"call_id":"…"}` header, then sealed video chunks) but a separate
/// protocol so the swarm routes audio and video to their own engines.
/// See `docs/GHAL_BOL_VIDEO_NATIVE_V1.md`.
pub(crate) const CALL_VIDEO_STREAM_PROTOCOL: &str = "/ghal-bol/call-video/1.0.0";

