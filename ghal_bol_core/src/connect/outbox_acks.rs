//! Outbox drain + delivery/read ack dispatch for connect sessions.

use std::sync::Arc;

use super::chat_room_session::read_ack_cutoff_ms;
use super::frames::{
    build_call_signal_frame, build_pending_outbound_frame, new_ack_msg_id, send_frame_to_peer,
};
use super::peer_session::{writer_open_for_peer, SessionWriters};
use super::prelude::*;
use super::session::{chrono_now_ms, SessionState};
use super::types::{
    GossipChatEvent, PendingOutbound, SessionPeer,
    OUTBOX_RESEND_INTERVAL_MS,
};
use super::ui_session::{may_send_in_room_read_ack, read_ack_catchup_throttled};

const READ_ACK_UPKEEP_MAX_OPS: usize = 32;
const ACK_BURST_MAX_ROUNDS: usize = 4;
const ACK_BURST_MAX_OPS: usize = 64;

pub(crate) async fn maybe_send_transport_kem_hello(
    session: &SessionState,
    peer: &SessionPeer,
    writers: &SessionWriters,
) {
    if session.transport_hello_already_sent(peer) {
        return;
    }
    let env = match build_transport_kem_hello_envelope(
        &format!("tkem-{}", &peer[..peer.len().min(16)]),
        &session.identity,
        peer,
        session.dm_local_transport_sk(),
        chrono_now_ms(),
    ) {
        Ok(e) => e,
        Err(e) => {
            native_log::debug("connect", format!("transport kem hello build: {e}"));
            return;
        }
    };
    let frame = match envelope_to_frame_bytes(&env) {
        Ok(f) => f,
        Err(e) => {
            native_log::debug("connect", format!("transport kem hello frame: {e}"));
            return;
        }
    };
    if send_frame_to_peer(peer, frame, writers).await.is_ok() {
        session.mark_transport_hello_sent(peer);
        native_log::info("connect", format!("transport kem hello sent to {peer}"));
    } else {
        native_log::warn(
            "connect",
            format!("transport kem hello send failed to {peer}"),
        );
    }
}

pub(crate) async fn send_ack_frame(
    peer: &SessionPeer,
    recipient_signing: &str,
    ref_id: &str,
    kind: MsgKind,
    session: &SessionState,
    writers: &SessionWriters,
    received_at_ms: Option<i64>,
) -> bool {
    let Ok(env) = build_ack_envelope(
        &new_ack_msg_id(),
        ref_id,
        kind,
        &session.identity,
        recipient_signing,
        chrono_now_ms(),
        received_at_ms,
    ) else {
        return false;
    };
    let Ok(frame) = envelope_to_frame_bytes(&env) else {
        return false;
    };
    send_frame_to_peer(peer, frame, writers).await.is_ok()
}

pub(crate) async fn send_inbound_delivery_ack(
    peer: &SessionPeer,
    inbound_id: &str,
    sender_signing: &str,
    session: &SessionState,
    writers: &SessionWriters,
) {
    if session.is_delivery_ack_sent(inbound_id) {
        return;
    }
    let received_at_ms = chrono_now_ms();
    if send_ack_frame(
        peer,
        sender_signing,
        inbound_id,
        MsgKind::AckReceived,
        session,
        writers,
        Some(received_at_ms),
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
    session.enqueue_delivery_ack(peer, inbound_id, sender_signing, received_at_ms);
}

pub(crate) async fn send_inbound_read_ack_if_possible(
    peer: &SessionPeer,
    inbound_id: &str,
    sender_signing: &str,
    session: &SessionState,
    writers: &SessionWriters,
) {
    if session.is_read_ack_confirmed(inbound_id) {
        return;
    }
    if !session.try_claim_read_ack_wire_send(peer, inbound_id) {
        return;
    }
    if send_ack_frame(
        peer,
        sender_signing,
        inbound_id,
        MsgKind::AckRead,
        session,
        writers,
        None,
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
}

pub(crate) fn dispatch_read_ack_pass(
    session: Arc<SessionState>,
    writers: SessionWriters,
    peer: SessionPeer,
    cutoff_ms: i64,
) {
    seed_read_acks_for_peer_from_transcript(session.as_ref(), &peer, cutoff_ms);
    tokio::spawn(async move {
        read_ack_catchup_for_peer(session, writers, peer).await;
    });
}

fn seed_read_acks_for_peer_from_transcript(
    session: &SessionState,
    peer: &SessionPeer,
    cutoff_ms: i64,
) {
    let Some(ns) = session
        .app_namespace
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
    else {
        return;
    };
    let lookup_keys =
        crate::dm_event_handler::inbound_transcript_lookup_keys(ns, peer, peer, peer);
    let Ok(rows) = crate::dm_transcript_store::load_merged(ns, &lookup_keys, Some(peer)) else {
        return;
    };
    let mut seeded = 0usize;
    for row in rows {
        if row.outgoing || row.read_ack_sent {
            continue;
        }
        let Some(received_at) = row.received_at_ms.or(row.created_at_ms) else {
            continue;
        };
        if received_at > cutoff_ms {
            continue;
        }
        let Some(message_id) = row.message_id.as_deref().map(str::trim).filter(|s| !s.is_empty())
        else {
            continue;
        };
        if session.enqueue_read_ack_backlog(peer, message_id) {
            seeded += 1;
        }
    }
    if seeded > 0 {
        native_log::info(
            "read_ack",
            format!("seeded {seeded} pending read ack(s) for {peer} cutoff_ms={cutoff_ms}"),
        );
    }
}

async fn read_ack_catchup_for_peer(
    session: Arc<SessionState>,
    writers: SessionWriters,
    peer: SessionPeer,
) {
    for _ in 0..80 {
        if writer_open_for_peer(&writers, &peer) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    if writer_open_for_peer(&writers, &peer) {
        run_ack_upkeep_burst(session, writers, peer).await;
    }
}

pub(crate) async fn run_ack_upkeep(
    session: Arc<SessionState>,
    writers: SessionWriters,
    connected_peers: &[SessionPeer],
) {
    if connected_peers.is_empty() {
        return;
    }
    for peer in connected_peers {
        let cutoff = read_ack_cutoff_ms(session.as_ref(), peer);
        if session.has_pending_read_acks_for(peer) {
            seed_read_acks_for_peer_from_transcript(session.as_ref(), peer, cutoff);
        }
    }
    run_ack_upkeep_limited(session, writers, connected_peers, READ_ACK_UPKEEP_MAX_OPS, true).await;
}

async fn run_ack_upkeep_limited(
    session: Arc<SessionState>,
    writers: SessionWriters,
    connected_peers: &[SessionPeer],
    limit: usize,
    include_read: bool,
) -> usize {
    let connected: HashSet<SessionPeer> = connected_peers.iter().cloned().collect();
    let mut done = 0usize;
    for item in session.delivery_acks_due_for_upkeep(limit) {
        if !connected.contains(&item.peer) {
            continue;
        }
        if send_ack_frame(
            &item.peer,
            &item.recipient_public_key_hex,
            &item.inbound_id,
            MsgKind::AckReceived,
            session.as_ref(),
            &writers,
            Some(item.received_at_ms),
        )
        .await
        {
            session.mark_delivery_ack_sent(&item.inbound_id);
            session.dequeue_delivery_ack(&item.inbound_id);
            done += 1;
        }
    }
    if include_read {
        for item in session.read_acks_due_for_upkeep(limit) {
            if !connected.contains(&item.peer) {
                continue;
            }
            if !may_send_in_room_read_ack(session.as_ref(), &item.peer)
                && super::ui_session::is_live_foreground_peer(&item.peer)
            {
                continue;
            }
            if session.is_read_ack_confirmed(&item.inbound_id) {
                continue;
            }
            if send_ack_frame(
                &item.peer,
                &item.recipient_public_key_hex,
                &item.inbound_id,
                MsgKind::AckRead,
                session.as_ref(),
                &writers,
                None,
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

pub(crate) async fn run_ack_upkeep_burst(
    session: Arc<SessionState>,
    writers: SessionWriters,
    peer: SessionPeer,
) {
    let connected = [peer.clone()];
    for round in 0..ACK_BURST_MAX_ROUNDS {
        let before = session.pending_read_ack_len();
        let n = run_ack_upkeep_limited(
            Arc::clone(&session),
            Arc::clone(&writers),
            &connected,
            ACK_BURST_MAX_OPS,
            true,
        )
        .await;
        if n == 0 && session.pending_read_ack_len() == before {
            break;
        }
        if round + 1 >= ACK_BURST_MAX_ROUNDS {
            break;
        }
    }
}

pub(crate) async fn resync_pending_outbox(
    session: Arc<SessionState>,
    writers: SessionWriters,
    connected_peers: Vec<SessionPeer>,
    events_tx: Option<std::sync::mpsc::Sender<GossipChatEvent>>,
) {
    let now = chrono_now_ms();
    let due: Vec<_> = session
        .outbox_due_for_resend(now)
        .into_iter()
        .filter(|p| connected_peers.iter().any(|cp| cp == &p.peer))
        .collect();
    if !due.is_empty() {
        native_log::debug("outbox", format!("resync {} pending message(s)", due.len()));
    }
    for p in due {
        if !writer_open_for_peer(&writers, &p.peer) {
            continue;
        }
        let frame = match build_pending_outbound_frame(session.as_ref(), &p) {
            Ok(f) => f,
            Err(e) => {
                native_log::warn("outbox", format!("resync skip msg_id={}: {e}", p.message_id));
                continue;
            }
        };
        if send_frame_to_peer(&p.peer, frame, &writers).await.is_ok() {
            if session.mark_outbox_sent(&p.message_id, now) {
                if let Some(tx) = &events_tx {
                    let _ = tx.send(GossipChatEvent::OutboundSent {
                        message_id: p.message_id.clone(),
                    });
                }
            }
        } else {
            session.mark_outbox_send_failed(&p.message_id, now);
        }
    }
}

pub(crate) async fn resync_outbox_burst_for_peer(
    session: Arc<SessionState>,
    writers: SessionWriters,
    peer: SessionPeer,
    events_tx: Option<std::sync::mpsc::Sender<GossipChatEvent>>,
) {
    let now = chrono_now_ms();
    let rows: Vec<PendingOutbound> = session.pending_outbox_for_peer(&peer)
        .into_iter()
        .filter(|p| {
            !p.on_wire || now.saturating_sub(p.last_send_ms) >= OUTBOX_RESEND_INTERVAL_MS
        })
        .collect();
    if rows.is_empty() {
        return;
    }
    native_log::info(
        "outbox",
        format!("burst resync {} pending row(s) to {peer}", rows.len()),
    );
    for p in rows {
        if !writer_open_for_peer(&writers, &peer) {
            break;
        }
        let frame = match build_pending_outbound_frame(session.as_ref(), &p) {
            Ok(f) => f,
            Err(_) => continue,
        };
        if send_frame_to_peer(&peer, frame, &writers).await.is_ok() {
            if session.mark_outbox_sent(&p.message_id, now) {
                if let Some(tx) = &events_tx {
                    let _ = tx.send(GossipChatEvent::OutboundSent {
                        message_id: p.message_id.clone(),
                    });
                }
            }
        }
    }
}

pub(crate) async fn flush_pending_call_signals(
    session: Arc<SessionState>,
    writers: SessionWriters,
    connected_peers: Vec<SessionPeer>,
    events_tx: Option<std::sync::mpsc::Sender<GossipChatEvent>>,
) {
    let batch = session.drain_pending_call_signals(32);
    for call in batch {
        if !connected_peers.iter().any(|p| p == &call.peer) {
            session.requeue_pending_call_signal_front(call);
            continue;
        }
        if !writer_open_for_peer(&writers, &call.peer) {
            session.requeue_pending_call_signal_front(call);
            continue;
        }
        let frame = match build_call_signal_frame(session.as_ref(), &call) {
            Ok(f) => f,
            Err(e) => {
                native_log::info("call", format!("call signal deferred: {e}"));
                session.requeue_pending_call_signal_front(call);
                continue;
            }
        };
        match send_frame_to_peer(&call.peer, frame, &writers).await {
            Ok(()) => {
                if let Some(tx) = events_tx.as_ref() {
                    let _ = tx.send(GossipChatEvent::CallSignalSent {
                        call_id: call.call_id.clone(),
                        signal: call.signal_kind.wire_name().to_string(),
                        recipient_public_key_hex: call.recipient_public_key_hex.clone(),
                    });
                }
            }
            Err(e) => {
                native_log::warn("call", format!("call send failed: {e}"));
                session.requeue_pending_call_signal_front(call);
            }
        }
    }
}

pub(crate) async fn handle_run_read_ack_catchup(
    session: Arc<SessionState>,
    writers: SessionWriters,
    identity_wire: String,
) {
    let Ok(peer) = super::types::session_peer_from_identity_wire(&identity_wire) else {
        return;
    };
    let now = chrono_now_ms();
    if read_ack_catchup_throttled(&peer, now) {
        return;
    }
    let cutoff = read_ack_cutoff_ms(session.as_ref(), &peer);
    dispatch_read_ack_pass(Arc::clone(&session), writers, peer, cutoff);
}

pub(crate) async fn run_ack_upkeep_limited_delivery(
    session: Arc<SessionState>,
    writers: SessionWriters,
    connected_peers: &[SessionPeer],
) {
    run_ack_upkeep_limited(session, writers, connected_peers, READ_ACK_UPKEEP_MAX_OPS, false).await;
}

pub(crate) async fn handle_send_ack_cmd(
    session: Arc<SessionState>,
    writers: SessionWriters,
    recipient_public_key_hex: String,
    ref_id: String,
    ack_kind: MsgKind,
) {
    let Ok(peer) = super::types::session_peer_from_identity_wire(&recipient_public_key_hex) else {
        return;
    };
    let _ = send_ack_frame(
        &peer,
        &recipient_public_key_hex,
        &ref_id,
        ack_kind,
        session.as_ref(),
        &writers,
        None,
    )
    .await;
}
