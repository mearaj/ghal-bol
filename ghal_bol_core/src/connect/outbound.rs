//! Process outbound commands for the connect worker.

use std::sync::Arc;

use super::bridge_ws::{bridge_request_for_call, connect_bridge_session};
use super::chat_room_session::begin_chat_room_session;
use super::frames::{
    build_pending_outbound_frame, send_frame_to_peer, start_call_media_for_peer,
    start_call_video_for_peer,
};
use super::outbox_acks::{
    flush_pending_call_signals, handle_run_read_ack_catchup, handle_send_ack_cmd,
};
use super::peer_session::writer_open_for_peer;
use super::prelude::*;
use super::session::chrono_now_ms;
use super::types::{
    session_peer_from_identity_wire, GossipChatEvent, OutboundCmd, PendingCallSignal,
    PendingOutbound,
};
use super::ui_session::{emit_call_media, on_local_call_signal_sent};
use super::worker::ConnectWorkerState;

pub(crate) async fn process_outbound_cmds(
    worker: Arc<ConnectWorkerState>,
    events_tx: &Option<std::sync::mpsc::Sender<GossipChatEvent>>,
    outbound_rx: &std::sync::mpsc::Receiver<OutboundCmd>,
    max: usize,
) {
    for _ in 0..max {
        let Ok(cmd) = outbound_rx.try_recv() else {
            break;
        };
        process_one(Arc::clone(&worker), events_tx, cmd).await;
    }
}

async fn process_one(
    worker: Arc<ConnectWorkerState>,
    events_tx: &Option<std::sync::mpsc::Sender<GossipChatEvent>>,
    cmd: OutboundCmd,
) {
    let session = Arc::clone(&worker.session);
    let writers = Arc::clone(&worker.registry.writers);
    match cmd {
        OutboundCmd::RegisterDmPeer { public_key_hex } => {
            session.register_dm_peer_key(&public_key_hex);
        }
        OutboundCmd::SetForegroundPeer {
            identity_wire,
            generation: _,
        } => {
            let peer = identity_wire
                .as_deref()
                .and_then(|w| session_peer_from_identity_wire(w).ok());
            if let Some(ref p) = peer {
                begin_chat_room_session(session.as_ref(), p);
            }
            session.set_foreground_peer(peer);
        }
        OutboundCmd::RunReadAckCatchup { identity_wire } => {
            handle_run_read_ack_catchup(
                Arc::clone(&session),
                writers,
                identity_wire,
            )
            .await;
        }
        OutboundCmd::SendText {
            recipient_public_key_hex,
            text,
            message_id,
            created_at_ms,
            done,
        } => {
            let Ok(peer) = session_peer_from_identity_wire(&recipient_public_key_hex) else {
                let _ = done.map(|tx| tx.send(Err("invalid recipient".into())));
                return;
            };
            session.register_dm_peer_key(&recipient_public_key_hex);
            let now = chrono_now_ms();
            let pending = PendingOutbound {
                message_id: message_id.clone(),
                peer: peer.clone(),
                recipient_public_key_hex: recipient_public_key_hex.clone(),
                text: text.clone(),
                created_at_ms,
                last_send_ms: 0,
                first_on_wire_ms: 0,
                on_wire: false,
            };
            session.track_outbound(pending.clone());
            if writer_open_for_peer(&writers, &peer) {
                match build_pending_outbound_frame(session.as_ref(), &pending) {
                    Ok(frame) => {
                        if send_frame_to_peer(&peer, frame, &writers).await.is_ok() {
                            session.mark_outbox_sent(&message_id, now);
                            if let Some(tx) = events_tx {
                                let _ = tx.send(GossipChatEvent::OutboundSent {
                                    message_id: message_id.clone(),
                                });
                            }
                            let _ = done.map(|tx| tx.send(Ok(())));
                            return;
                        }
                    }
                    Err(e) => {
                        native_log::debug("outbound", format!("send_text deferred: {e}"));
                    }
                }
            }
            let _ = done.map(|tx| tx.send(Err("peer stream not ready".into())));
        }
        OutboundCmd::SendAck {
            recipient_public_key_hex,
            ref_id,
            ack_kind,
        } => {
            handle_send_ack_cmd(
                Arc::clone(&session),
                writers,
                recipient_public_key_hex,
                ref_id,
                ack_kind,
            )
            .await;
        }
        OutboundCmd::SendCallSignal {
            recipient_public_key_hex,
            call_id,
            signal_kind,
            payload,
            signal_id,
        } => {
            let Ok(peer) = session_peer_from_identity_wire(&recipient_public_key_hex) else {
                return;
            };
            let recipient_hex = recipient_public_key_hex.clone();
            if let Err(e) = call_state::apply_outbound(&recipient_hex, &call_id, signal_kind) {
                native_log::warn("call", format!("outbound signal rejected: {e}"));
                return;
            }
            on_local_call_signal_sent(&call_id, signal_kind);
            session.enqueue_pending_call_signal(PendingCallSignal {
                call_id: call_id.clone(),
                signal_id,
                signal_kind,
                payload,
                peer: peer.clone(),
                recipient_public_key_hex,
                created_at_ms: chrono_now_ms(),
            });
            if writer_open_for_peer(&writers, &peer) {
                flush_pending_call_signals(
                    Arc::clone(&session),
                    Arc::clone(&writers),
                    vec![peer.clone()],
                    events_tx.clone(),
                )
                .await;
            } else if let Ok(bridge) = bridge_request_for_call(&recipient_hex, &call_id) {
                let reg = Arc::clone(&worker.registry);
                let sess = Arc::clone(&session);
                let id = worker.identity.clone();
                let ev = worker.events_tx.clone();
                let pw = peer.clone();
                tokio::spawn(async move {
                    if let Err(e) = connect_bridge_session(reg, sess, id, ev, pw, bridge).await {
                        native_log::warn("bridge", format!("wan call bridge failed: {e}"));
                    }
                });
            }
            if let Some(tx) = events_tx {
                let _ = tx.send(GossipChatEvent::CallSignalSent {
                    call_id,
                    signal: signal_kind.wire_name().to_string(),
                    recipient_public_key_hex: recipient_hex,
                });
            }
        }
        OutboundCmd::CallMediaStart {
            call_id,
            peer_public_key_hex,
        } => {
            if let Err(e) = start_call_media_for_peer(
                Arc::clone(&session),
                writers,
                call_id.clone(),
                peer_public_key_hex,
                events_tx.clone(),
            )
            .await
            {
                native_log::warn("call_media", format!("start failed call_id={call_id}: {e}"));
            }
        }
        OutboundCmd::CallMediaStop { call_id } => {
            let stopped = session.call_media_stop(&call_id);
            if stopped {
                if let Some(pk) = crate::p2p::call_active::snapshot().map(|s| s.peer_public_key_hex) {
                    emit_call_media(events_tx, &call_id, &pk, "voice_stopped", None);
                }
            }
            #[cfg(target_os = "android")]
            crate::call_media::reset_voice_audio_mode_flag();
        }
        OutboundCmd::CallMediaSetMicMuted { call_id, muted } => {
            session.call_media_set_mic_muted(&call_id, muted);
        }
        OutboundCmd::CallMediaSetSpeaker { call_id, speaker_on } => {
            #[cfg(not(target_os = "android"))]
            let _ = crate::call_media::set_speakerphone(speaker_on);
            #[cfg(target_os = "android")]
            let _ = crate::call_media::set_speakerphone(speaker_on);
            let _ = call_id;
        }
        OutboundCmd::CallVideoStart {
            call_id,
            peer_public_key_hex,
            camera_enabled,
        } => {
            if let Err(e) = start_call_video_for_peer(
                Arc::clone(&session),
                writers,
                call_id.clone(),
                peer_public_key_hex,
                camera_enabled,
                events_tx.clone(),
            )
            .await
            {
                native_log::warn("call_video", format!("start failed call_id={call_id}: {e}"));
            }
        }
        OutboundCmd::CallVideoStop { call_id } => {
            session.call_video_stop(&call_id);
            if let Some(pk) = crate::p2p::call_active::snapshot().map(|s| s.peer_public_key_hex) {
                emit_call_media(events_tx, &call_id, &pk, "video_stopped", None);
            }
        }
        OutboundCmd::CallVideoSetCameraEnabled { call_id, enabled } => {
            session.call_video_set_camera_off(&call_id, !enabled);
        }
    }
}
