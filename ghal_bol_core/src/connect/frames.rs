//! Channel 0 frame dispatch + outbound frame builders.

use std::sync::Arc;

use super::channel_mux::{CHANNEL_ATTACH, CHANNEL_CALL_AUDIO, CHANNEL_CALL_VIDEO, CHANNEL_MSG};
use super::notify::drop_pending_call_invite;
use super::outbox_acks::{
    flush_pending_call_signals, maybe_send_transport_kem_hello, resync_outbox_burst_for_peer,
    resync_pending_outbox, run_ack_upkeep_burst, send_ack_frame, send_inbound_delivery_ack,
    send_inbound_read_ack_if_possible,
};
use super::peer_session::{
    SessionWriters, queue_frame_for_peer, queue_mux_for_peer, writer_open_for_peer,
};
use super::prelude::*;
use super::session::{SessionState, chrono_now_ms};
use super::types::{GossipChatEvent, PendingCallSignal, PendingOutbound, SessionPeer};
use super::ui_session::{
    emit_call_media, may_send_in_room_read_ack, platform_incoming_call_dismiss,
    platform_incoming_call_show,
};

pub(crate) struct WireDispatchCtx {
    pub session: Arc<SessionState>,
    pub events_tx: Option<std::sync::mpsc::Sender<GossipChatEvent>>,
    pub peer: SessionPeer,
    pub writers: SessionWriters,
}

pub(crate) async fn dispatch_mux_payload(channel: u32, payload: &[u8], ctx: &WireDispatchCtx) {
    match channel {
        CHANNEL_MSG => handle_channel0_frame(payload, ctx).await,
        CHANNEL_CALL_AUDIO => handle_call_media_payload(payload, ctx, false).await,
        CHANNEL_CALL_VIDEO => handle_call_media_payload(payload, ctx, true).await,
        CHANNEL_ATTACH => handle_attachment_payload(payload, ctx).await,
        _ => {}
    }
}

async fn handle_channel0_frame(payload: &[u8], ctx: &WireDispatchCtx) {
    let frame = payload.to_vec();
    let share = match frame_wire_share(&frame) {
        Ok(s) => s,
        Err(_) => return,
    };
    if share == CALL_SHARE {
        handle_inbound_call_frame(&frame, ctx).await;
        return;
    }
    if share != MSG_SHARE {
        return;
    }
    let env = match frame_bytes_to_envelope(&frame) {
        Ok(e) => e,
        Err(_) => return,
    };
    let identity = ctx.session.identity.clone();
    let peer_transport_pk = ctx
        .session
        .peer_transport_pk(env.sender_public_key_hex.trim());
    let open_transport = peer_transport_pk
        .as_ref()
        .map(|peer_pk| DmOpenTransportCtx {
            local_sk: ctx.session.dm_local_transport_sk(),
            peer_pk,
        });
    let parsed = match parse_envelope_with_transport(&env, &identity, open_transport) {
        Ok(p) => p,
        Err(e) => {
            native_log::warn("connect", format!("drop frame from {}: {e}", ctx.peer));
            return;
        }
    };
    match parsed {
        ParsedMsg::TransportKemHello {
            sender_public_key_hex,
            transport_pk,
        } => {
            ctx.session
                .store_peer_transport_pk(&sender_public_key_hex, transport_pk);
            super::transport_kem::set_peer_transport_pk(sender_public_key_hex.trim(), transport_pk);
            native_log::info(
                "connect",
                format!(
                    "transport kem hello from {} pk stored — flushing deferred",
                    ctx.peer
                ),
            );
            // Ensure our hello is on the wire too (initiator often never saw responder hello).
            maybe_send_transport_kem_hello(ctx.session.as_ref(), &ctx.peer, &ctx.writers).await;
            // Call invite (and sealed chat) wait on peer transport pk. Session-ready
            // flush often races before inbound hello arrives — re-flush now.
            flush_pending_call_signals(
                Arc::clone(&ctx.session),
                Arc::clone(&ctx.writers),
                vec![ctx.peer.clone()],
                ctx.events_tx.clone(),
            )
            .await;
            resync_outbox_burst_for_peer(
                Arc::clone(&ctx.session),
                Arc::clone(&ctx.writers),
                ctx.peer.clone(),
                ctx.events_tx.clone(),
            )
            .await;
        }
        ParsedMsg::Text(t) => {
            if !sender_matches_peer(&t.sender_public_key_hex, &ctx.peer) {
                native_log::warn(
                    "connect",
                    format!("drop text from {}: signing key mismatch", ctx.peer),
                );
                return;
            }
            native_log::info(
                "connect",
                format!(
                    "inbound text from {} id={} len={}",
                    ctx.peer,
                    t.id,
                    t.text.len()
                ),
            );
            let is_new = ctx.session.mark_seen_inbound(&t.id, chrono_now_ms());
            ctx.session.register_dm_peer_key(&t.sender_public_key_hex);
            let mut persisted_on_wire = false;
            if is_new {
                let received_at_ms = chrono_now_ms();
                persisted_on_wire = crate::dm_event_handler::persist_inbound_text_on_wire(
                    &ctx.peer,
                    &t.id,
                    &t.text,
                    &t.sender_public_key_hex,
                    t.created_at_ms,
                    received_at_ms,
                );
                if persisted_on_wire {
                    if let Some(tx) = &ctx.events_tx {
                        let _ = tx.send(GossipChatEvent::DmMessage {
                            from: ctx.peer.clone(),
                            id: t.id.clone(),
                            msg_kind: "text".to_string(),
                            text: Some(t.text.clone()),
                            ref_id: None,
                            sender_public_key_hex: t.sender_public_key_hex.clone(),
                            created_at_ms: t.created_at_ms,
                            received_at_ms: Some(received_at_ms),
                            duration_ms: None,
                        });
                    }
                }
            } else {
                ctx.session.clear_delivery_ack_sent(&t.id);
            }
            send_inbound_delivery_ack(
                &ctx.peer,
                &t.id,
                &t.sender_public_key_hex,
                ctx.session.as_ref(),
                &ctx.writers,
            )
            .await;
            let may_read = may_send_in_room_read_ack(ctx.session.as_ref(), &ctx.peer)
                && !ctx.session.is_read_ack_confirmed(&t.id)
                && (!is_new || persisted_on_wire);
            if may_read {
                send_inbound_read_ack_if_possible(
                    &ctx.peer,
                    &t.id,
                    &t.sender_public_key_hex,
                    ctx.session.as_ref(),
                    &ctx.writers,
                )
                .await;
            }
        }
        ParsedMsg::Voice(v) => {
            if !sender_matches_peer(&v.sender_public_key_hex, &ctx.peer) {
                native_log::warn(
                    "connect",
                    format!("drop voice from {}: signing key mismatch", ctx.peer),
                );
                return;
            }
            native_log::info(
                "connect",
                format!(
                    "inbound voice from {} id={} opus_len={} duration_ms={}",
                    ctx.peer,
                    v.id,
                    v.opus_blob.len(),
                    v.duration_ms
                ),
            );
            let is_new = ctx.session.mark_seen_inbound(&v.id, chrono_now_ms());
            ctx.session.register_dm_peer_key(&v.sender_public_key_hex);
            let mut persisted_on_wire = false;
            if is_new {
                let received_at_ms = chrono_now_ms();
                let Some(ns) = crate::dm_event_handler::active_app_namespace() else {
                    native_log::warn("connect", "drop voice: handler namespace not set");
                    return;
                };
                let audio_path =
                    match crate::voice_msg_v1::write_voice_audio_file(&ns, &v.id, &v.opus_blob) {
                        Ok(p) => p,
                        Err(e) => {
                            native_log::warn(
                                "connect",
                                format!("drop voice: persist audio failed: {e}"),
                            );
                            return;
                        }
                    };
                persisted_on_wire = crate::dm_event_handler::persist_inbound_voice_on_wire(
                    &ctx.peer,
                    &v.id,
                    v.duration_ms,
                    &audio_path,
                    &v.sender_public_key_hex,
                    v.created_at_ms,
                    received_at_ms,
                );
                if persisted_on_wire {
                    if let Some(tx) = &ctx.events_tx {
                        let _ = tx.send(GossipChatEvent::DmMessage {
                            from: ctx.peer.clone(),
                            id: v.id.clone(),
                            msg_kind: "voice".to_string(),
                            text: Some(crate::voice_msg_v1::voice_preview(v.duration_ms)),
                            ref_id: None,
                            sender_public_key_hex: v.sender_public_key_hex.clone(),
                            created_at_ms: v.created_at_ms,
                            received_at_ms: Some(received_at_ms),
                            duration_ms: Some(v.duration_ms),
                        });
                    }
                }
            } else {
                ctx.session.clear_delivery_ack_sent(&v.id);
            }
            send_inbound_delivery_ack(
                &ctx.peer,
                &v.id,
                &v.sender_public_key_hex,
                ctx.session.as_ref(),
                &ctx.writers,
            )
            .await;
            let may_read = may_send_in_room_read_ack(ctx.session.as_ref(), &ctx.peer)
                && !ctx.session.is_read_ack_confirmed(&v.id)
                && (!is_new || persisted_on_wire);
            if may_read {
                send_inbound_read_ack_if_possible(
                    &ctx.peer,
                    &v.id,
                    &v.sender_public_key_hex,
                    ctx.session.as_ref(),
                    &ctx.writers,
                )
                .await;
            }
        }
        ParsedMsg::AttachmentOffer(a) => {
            if !sender_matches_peer(&a.sender_public_key_hex, &ctx.peer) {
                native_log::warn(
                    "connect",
                    format!(
                        "drop attachment offer from {}: signing key mismatch",
                        ctx.peer
                    ),
                );
                return;
            }
            let local_path = if let Some(plain) = a.file_plain.as_ref() {
                native_log::info(
                    "attach",
                    format!(
                        "inbound mailbox attachment from {} id={} name={} bytes={}",
                        ctx.peer, a.id, a.file_name, a.size_plaintext
                    ),
                );
                match ctx.session.app_namespace.as_deref() {
                    Some(ns) => crate::attach_v1::write_attachment_file(
                        ns,
                        &a.id,
                        &a.file_name,
                        plain,
                    )
                    .ok(),
                    None => None,
                }
            } else {
                native_log::info(
                    "attach",
                    format!(
                        "inbound LAN mux attachment offer from {} id={} blob={} name={} bytes={}",
                        ctx.peer, a.id, a.blob_id, a.file_name, a.size_plaintext
                    ),
                );
                crate::attach_v1::remember_offer(crate::attach_v1::AttachmentOfferMeta {
                    id: a.id.clone(),
                    sender_public_key_hex: a.sender_public_key_hex.clone(),
                    blob_id: a.blob_id.clone(),
                    file_name: a.file_name.clone(),
                    mime_type: a.mime_type.clone(),
                    size_plaintext: a.size_plaintext,
                    sha256_plaintext: a.sha256_plaintext.clone(),
                    content_key_b64: a.content_key_b64.clone(),
                    expires_at_ms: a.expires_at_ms,
                });
                None
            };
            let is_new = ctx.session.mark_seen_inbound(&a.id, chrono_now_ms());
            ctx.session.register_dm_peer_key(&a.sender_public_key_hex);
            let mut persisted_on_wire = false;
            if is_new {
                let received_at_ms = chrono_now_ms();
                persisted_on_wire =
                    crate::dm_event_handler::persist_inbound_attachment_offer_on_wire(
                        &ctx.peer,
                        &a.id,
                        &a.file_name,
                        &a.mime_type,
                        a.size_plaintext,
                        &a.sender_public_key_hex,
                        a.created_at_ms,
                        received_at_ms,
                        local_path.as_deref(),
                    );
                if persisted_on_wire {
                    if let Some(tx) = &ctx.events_tx {
                        let _ = tx.send(GossipChatEvent::DmMessage {
                            from: ctx.peer.clone(),
                            id: a.id.clone(),
                            msg_kind: "attachment_offer".to_string(),
                            text: Some(crate::attach_v1::attachment_preview(&a.file_name)),
                            ref_id: None,
                            sender_public_key_hex: a.sender_public_key_hex.clone(),
                            created_at_ms: a.created_at_ms,
                            received_at_ms: Some(received_at_ms),
                            duration_ms: None,
                        });
                    }
                }
            } else {
                ctx.session.clear_delivery_ack_sent(&a.id);
            }
            send_inbound_delivery_ack(
                &ctx.peer,
                &a.id,
                &a.sender_public_key_hex,
                ctx.session.as_ref(),
                &ctx.writers,
            )
            .await;
            let may_read = may_send_in_room_read_ack(ctx.session.as_ref(), &ctx.peer)
                && !ctx.session.is_read_ack_confirmed(&a.id)
                && (!is_new || persisted_on_wire);
            if may_read {
                send_inbound_read_ack_if_possible(
                    &ctx.peer,
                    &a.id,
                    &a.sender_public_key_hex,
                    ctx.session.as_ref(),
                    &ctx.writers,
                )
                .await;
            }
        }
        ParsedMsg::AvailabilityStatus(s) => {
            if !sender_matches_peer(&s.sender_public_key_hex, &ctx.peer) {
                native_log::warn(
                    "connect",
                    format!(
                        "drop availability_status from {}: signing key mismatch",
                        ctx.peer
                    ),
                );
                return;
            }
            ctx.session.register_dm_peer_key(&s.sender_public_key_hex);
            let changed = crate::dm_event_handler::apply_p2p_event_json(&serde_json::json!({
                "kind": "dm_message",
                "from": ctx.peer.clone(),
                "id": s.id,
                "msg_kind": "availability_status",
                "sender_public_key_hex": s.sender_public_key_hex,
                "created_at_ms": s.created_at_ms,
                "updated_at_ms": s.updated_at_ms,
                "status": s.status.as_deref().unwrap_or_default(),
            }));
            native_log::info(
                "connect",
                format!("availability_status from {} changed={changed}", ctx.peer),
            );
            if changed {
                if let Some(tx) = &ctx.events_tx {
                    let status_text = s.status.clone().unwrap_or_default();
                    let _ = tx.send(GossipChatEvent::DmMessage {
                        from: ctx.peer.clone(),
                        id: format!("status-{}", chrono_now_ms()),
                        msg_kind: "availability_status".to_string(),
                        text: Some(status_text),
                        ref_id: None,
                        sender_public_key_hex: ctx.peer.clone(),
                        created_at_ms: chrono_now_ms(),
                        received_at_ms: None,
                        duration_ms: None,
                    });
                }
            }
        }
        ParsedMsg::Ack(a) => {
            if !sender_matches_peer(&a.sender_public_key_hex, &ctx.peer) {
                return;
            }
            if a.kind == MsgKind::AckRequest {
                return;
            }
            if a.kind == MsgKind::AttachmentComplete {
                let _ = crate::attach_v1::complete_serve(&a.ref_id, &ctx.peer);
                ctx.session.finalize_outbound_ack(&a.ref_id);
            }
            if a.kind == MsgKind::AckReceived {
                if ctx.session.has_pending_read_ack(&a.ref_id) {
                    ctx.session.mark_read_ack_confirmed(&a.ref_id);
                    return;
                }
                ctx.session.finalize_outbound_ack(&a.ref_id);
            }
            if a.kind == MsgKind::AckRead {
                ctx.session.finalize_outbound_ack(&a.ref_id);
                let _ = send_ack_frame(
                    &ctx.peer,
                    &a.sender_public_key_hex,
                    &a.ref_id,
                    MsgKind::AckReceived,
                    ctx.session.as_ref(),
                    &ctx.writers,
                    None,
                )
                .await;
            }
            let kind = match a.kind {
                MsgKind::AckReceived => "ack_received",
                MsgKind::AckRead => "ack_read",
                MsgKind::AttachmentComplete => "attachment_complete",
                _ => return,
            };
            native_log::info(
                "connect",
                format!("{kind} from {} ref={}", ctx.peer, a.ref_id),
            );
            if let Some(tx) = &ctx.events_tx {
                let _ = tx.send(GossipChatEvent::DmMessage {
                    from: ctx.peer.clone(),
                    id: a.id.clone(),
                    msg_kind: kind.to_string(),
                    text: None,
                    ref_id: Some(a.ref_id.clone()),
                    sender_public_key_hex: a.sender_public_key_hex.clone(),
                    created_at_ms: a.created_at_ms,
                    received_at_ms: a.received_at_ms,
                    duration_ms: None,
                });
            }
        }
    }
}

async fn handle_attachment_payload(payload: &[u8], ctx: &WireDispatchCtx) {
    let action = serde_json::from_slice::<serde_json::Value>(payload)
        .ok()
        .and_then(|v| v.get("action").and_then(|x| x.as_str()).map(str::to_string))
        .unwrap_or_default();
    if action == "fetch" {
        let blob_id = serde_json::from_slice::<serde_json::Value>(payload)
            .ok()
            .and_then(|v| {
                v.get("blob_id")
                    .and_then(|x| x.as_str())
                    .map(str::to_string)
            })
            .unwrap_or_default();
        let offset = serde_json::from_slice::<serde_json::Value>(payload)
            .ok()
            .and_then(|v| v.get("offset").and_then(|x| x.as_u64()))
            .unwrap_or(0);
        for frame in crate::attach_v1::serve_fetch_frames(&ctx.peer, &blob_id, offset) {
            let _ = queue_mux_for_peer(&ctx.writers, &ctx.peer, CHANNEL_ATTACH, frame);
        }
        return;
    }
    if let Some((offer_id, local_path)) =
        crate::attach_v1::handle_fetch_response(&ctx.peer, payload)
    {
        native_log::info(
            "attach",
            format!("attachment fetch complete offer={offer_id} path={local_path}"),
        );
        if let Some(ns) = ctx.session.app_namespace.as_deref() {
            let _ = crate::dm_transcript_store::patch_attachment_local_path(
                ns,
                &ctx.peer,
                &offer_id,
                &local_path,
            );
        }
        send_ack_frame(
            &ctx.peer,
            &ctx.peer,
            &offer_id,
            MsgKind::AttachmentComplete,
            ctx.session.as_ref(),
            &ctx.writers,
            None,
        )
        .await;
        if let Some(tx) = &ctx.events_tx {
            let _ = tx.send(GossipChatEvent::DmMessage {
                from: ctx.peer.clone(),
                id: format!("attachment_complete:{offer_id}"),
                msg_kind: "attachment_complete".to_string(),
                text: None,
                ref_id: Some(offer_id),
                sender_public_key_hex: ctx.peer.clone(),
                created_at_ms: chrono_now_ms(),
                received_at_ms: None,
                duration_ms: None,
            });
        }
    }
}

async fn handle_inbound_call_frame(frame: &[u8], ctx: &WireDispatchCtx) {
    let env = match call_envelope_from_frame(frame) {
        Ok(e) => e,
        Err(_) => return,
    };
    let identity = ctx.session.identity.clone();
    let peer_transport_pk = ctx
        .session
        .peer_transport_pk(env.sender_public_key_hex.trim());
    let parsed = match parse_call_envelope_with_transport(
        &env,
        &identity,
        peer_transport_pk
            .as_ref()
            .map(|peer_pk| crate::call_sig_v1::CallOpenTransportCtx {
                local_sk: ctx.session.dm_local_transport_sk(),
                peer_pk,
            }),
    ) {
        Ok(p) => p,
        Err(e) => {
            native_log::warn("call", format!("drop call frame from {}: {e}", ctx.peer));
            return;
        }
    };
    if !sender_matches_peer(&parsed.sender_public_key_hex, &ctx.peer) {
        return;
    }
    if matches!(parsed.kind, CallSigKind::Invite) {
        let now_ms = chrono_now_ms();
        if !call_invite_is_live(parsed.created_at_ms, now_ms) {
            drop_pending_call_invite(&parsed.call_id);
            call_state::clear_peer(&parsed.sender_public_key_hex);
            platform_incoming_call_dismiss();
            return;
        }
    }
    if call_state::apply_inbound(&parsed.sender_public_key_hex, &parsed.call_id, parsed.kind)
        .is_err()
    {
        return;
    }
    match parsed.kind {
        CallSigKind::Invite => {
            let media_up = crate::p2p::call_active::snapshot().is_some();
            let phase = call_state::peer_call_phase(&parsed.sender_public_key_hex);
            if !media_up && phase == call_state::CallPhase::IncomingRinging {
                platform_incoming_call_show(&parsed.sender_public_key_hex, &parsed.call_id);
            }
        }
        CallSigKind::Accept => platform_incoming_call_dismiss(),
        CallSigKind::VideoOn => {
            platform_incoming_call_dismiss();
            crate::p2p::call_active::set_remote_video_on(&parsed.call_id, true);
            emit_call_media(
                &ctx.events_tx,
                &parsed.call_id,
                &parsed.sender_public_key_hex,
                "remote_video_on",
                None,
            );
        }
        CallSigKind::VideoOff => {
            platform_incoming_call_dismiss();
            crate::p2p::call_active::set_remote_video_on(&parsed.call_id, false);
            emit_call_media(
                &ctx.events_tx,
                &parsed.call_id,
                &parsed.sender_public_key_hex,
                "remote_video_off",
                None,
            );
        }
        CallSigKind::Hangup | CallSigKind::Reject => {
            platform_incoming_call_dismiss();
            drop_pending_call_invite(&parsed.call_id);
            let cid = parsed.call_id.clone();
            if crate::p2p::call_active::snapshot().is_some_and(|s| s.call_id == cid) {
                ctx.session.call_media_stop(&cid);
                ctx.session.call_video_stop(&cid);
                crate::p2p::call_active::clear();
                emit_call_media(
                    &ctx.events_tx,
                    &cid,
                    &parsed.sender_public_key_hex,
                    "call_ended",
                    Some("remote_hangup"),
                );
            }
        }
        _ => {}
    }
    ctx.session
        .register_dm_peer_key(&parsed.sender_public_key_hex);
    if let Some(tx) = &ctx.events_tx {
        let _ = tx.send(GossipChatEvent::CallSignal {
            from: ctx.peer.clone(),
            id: parsed.id.clone(),
            call_id: parsed.call_id.clone(),
            signal: parsed.kind.wire_name().to_string(),
            sender_public_key_hex: parsed.sender_public_key_hex.clone(),
            created_at_ms: parsed.created_at_ms,
            payload: parsed.payload.clone(),
        });
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct CallStreamHeader {
    call_id: String,
}

async fn handle_call_media_payload(payload: &[u8], ctx: &WireDispatchCtx, video: bool) {
    if let Ok(header) = serde_json::from_slice::<CallStreamHeader>(payload) {
        let _ = header;
        return;
    }
    if video {
        if let Some(tx) = ctx.session.call_video_wire_in_any(&ctx.peer) {
            let _ = tx.try_send(payload.to_vec());
        }
    } else if let Some(tx) = ctx.session.call_media_wire_in_any(&ctx.peer) {
        let _ = tx.try_send(payload.to_vec());
    }
}

fn sender_matches_peer(sender_pk: &str, peer: &SessionPeer) -> bool {
    crate::public_key_util::same_contact_pk(sender_pk, peer)
}

pub(crate) fn new_ack_msg_id() -> String {
    format!(
        "ack-{}",
        super::types::new_msg_id_for_ffi().trim_start_matches("msg-")
    )
}

pub(crate) fn build_pending_outbound_frame(
    session: &SessionState,
    p: &PendingOutbound,
) -> Result<Vec<u8>, String> {
    let recipient_wire =
        crate::public_key_util::normalize_contact_identity_wire(&p.recipient_public_key_hex)?;
    let peer_transport_pk = session
        .peer_transport_pk(&recipient_wire)
        .ok_or_else(|| "transport kem not ready for peer".to_string())?;
    let transport = DmSealTransportCtx {
        local_sk: session.dm_local_transport_sk(),
        peer_pk: &peer_transport_pk,
    };
    let ts = if p.created_at_ms > 0 {
        p.created_at_ms
    } else {
        chrono_now_ms()
    };
    let env = if p.msg_kind.trim() == "voice" || p.voice_opus.is_some() {
        let opus = p
            .voice_opus
            .as_deref()
            .ok_or_else(|| "voice pending missing opus blob".to_string())?;
        let duration_ms = p
            .duration_ms
            .ok_or_else(|| "voice pending missing duration_ms".to_string())?;
        build_voice_envelope(
            &p.message_id,
            &session.identity,
            &recipient_wire,
            duration_ms,
            opus,
            ts,
            transport,
        )?
    } else if p.msg_kind.trim() == "attachment_offer" {
        let offer = p
            .attachment_offer
            .as_ref()
            .ok_or_else(|| "attachment pending missing offer".to_string())?;
        build_attachment_offer_envelope(
            &p.message_id,
            &session.identity,
            &recipient_wire,
            offer,
            ts,
            transport,
        )?
    } else {
        build_text_envelope(
            &p.message_id,
            &session.identity,
            &recipient_wire,
            &p.text,
            ts,
            transport,
        )?
    };
    envelope_to_frame_bytes(&env)
}

pub(crate) fn current_local_availability_status(session: &SessionState) -> Option<Option<String>> {
    let ns = session.app_namespace.as_deref()?.trim();
    if ns.is_empty() {
        return None;
    }
    let cfg = crate::app_paths::storage_config_for_namespace(ns);
    crate::preferences_v1::availability_status_get(&cfg)
        .ok()
        .map(|s| s.and_then(|v| crate::preferences_v1::sanitize_peer_display_alias(&v)))
}

pub(crate) async fn send_availability_status_to_peer(
    session: &SessionState,
    writers: &SessionWriters,
    peer: &SessionPeer,
    status: Option<&str>,
    updated_at_ms: i64,
) -> Result<(), String> {
    let recipient_wire = crate::public_key_util::normalize_contact_identity_wire(peer)?;
    let peer_transport_pk = session
        .peer_transport_pk(&recipient_wire)
        .ok_or_else(|| "transport kem not ready for peer".to_string())?;
    let transport = DmSealTransportCtx {
        local_sk: session.dm_local_transport_sk(),
        peer_pk: &peer_transport_pk,
    };
    let env = build_availability_status_envelope(
        &format!(
            "status-{}",
            super::types::new_msg_id_for_ffi().trim_start_matches("msg-")
        ),
        &session.identity,
        &recipient_wire,
        status,
        updated_at_ms,
        chrono_now_ms(),
        transport,
    )?;
    let frame = envelope_to_frame_bytes(&env)?;
    send_frame_to_peer(peer, frame, writers).await
}

pub(crate) async fn fetch_attachment_for_peer(
    session: Arc<SessionState>,
    writers: SessionWriters,
    peer: SessionPeer,
    offer_id: String,
    save_path: Option<String>,
    done: std::sync::mpsc::Sender<Result<String, String>>,
) -> Result<(), String> {
    let Some(ns) = session.app_namespace.as_deref() else {
        let _ = done.send(Err("app namespace not set".to_string()));
        return Err("app namespace not set".to_string());
    };
    if !writer_open_for_peer(&writers, &peer) {
        // LAN mux only — oversized local-network transfers. WAN uses delivery mailbox.
        let e = "no LAN connect session — large attachments need a live LAN peer link"
            .to_string();
        let _ = done.send(Err(e.clone()));
        return Err(e);
    }
    let (blob_id, req) = match crate::attach_v1::start_fetch(
        ns,
        &peer,
        &offer_id,
        save_path.as_deref(),
        done.clone(),
    ) {
        Ok(r) => r,
        Err(e) => {
            let _ = done.send(Err(e.clone()));
            return Err(e);
        }
    };
    if let Err(e) = queue_mux_for_peer(&writers, &peer, CHANNEL_ATTACH, req) {
        crate::attach_v1::fail_fetch(&blob_id, &e);
        return Err(e);
    }
    Ok(())
}

pub(crate) fn build_call_signal_frame(
    session: &SessionState,
    pending: &PendingCallSignal,
) -> Result<Vec<u8>, String> {
    use crate::call_sig_v1::{
        CallSealTransportCtx, build_call_envelope, call_envelope_to_frame_bytes,
    };
    let recipient =
        crate::public_key_util::normalize_contact_identity_wire(&pending.recipient_public_key_hex)?;
    let peer_pk = session
        .peer_transport_pk(&recipient)
        .ok_or_else(|| "transport kem not ready for peer".to_string())?;
    let transport = CallSealTransportCtx {
        local_sk: session.dm_local_transport_sk(),
        peer_pk: &peer_pk,
    };
    let env = build_call_envelope(
        &pending.signal_id,
        &pending.call_id,
        pending.signal_kind,
        &session.identity,
        &recipient,
        pending.payload.clone(),
        pending.created_at_ms,
        transport,
    )?;
    call_envelope_to_frame_bytes(&env)
}

pub(crate) async fn send_frame_to_peer(
    peer: &SessionPeer,
    frame: Vec<u8>,
    writers: &SessionWriters,
) -> Result<(), String> {
    queue_frame_for_peer(writers, peer, frame)
}

pub(crate) async fn on_session_ready(
    session: Arc<SessionState>,
    writers: SessionWriters,
    peer: SessionPeer,
    events_tx: Option<std::sync::mpsc::Sender<GossipChatEvent>>,
) {
    if !writer_open_for_peer(&writers, &peer) {
        return;
    }
    native_log::info("connect", format!("session ready {peer}"));
    let session2 = Arc::clone(&session);
    let writers2 = Arc::clone(&writers);
    let peer2 = peer.clone();
    tokio::spawn(async move {
        maybe_send_transport_kem_hello(session2.as_ref(), &peer2, &writers2).await;
        // Identity-derived peer pk is always available; flush invites immediately.
        flush_pending_call_signals(
            Arc::clone(&session2),
            writers2.clone(),
            vec![peer2.clone()],
            events_tx.clone(),
        )
        .await;
        resync_outbox_burst_for_peer(
            Arc::clone(&session2),
            writers2.clone(),
            peer2.clone(),
            events_tx.clone(),
        )
        .await;
        resync_pending_outbox(
            Arc::clone(&session2),
            writers2.clone(),
            vec![peer2.clone()],
            events_tx.clone(),
        )
        .await;
        if let Some(Some(status)) = current_local_availability_status(session2.as_ref()) {
            if let Err(e) = send_availability_status_to_peer(
                session2.as_ref(),
                &writers2,
                &peer2,
                Some(&status),
                chrono_now_ms(),
            )
            .await
            {
                native_log::debug("connect", format!("availability_status ready send: {e}"));
            }
        }
        if session2.has_pending_read_acks_for(&peer2) {
            run_ack_upkeep_burst(Arc::clone(&session2), writers2, peer2).await;
        }
    });
}

pub(crate) async fn start_call_media_for_peer(
    session: Arc<SessionState>,
    writers: SessionWriters,
    call_id: String,
    peer_public_key_hex: String,
    events_tx: Option<std::sync::mpsc::Sender<GossipChatEvent>>,
) -> Result<(), String> {
    let pk = peer_public_key_hex.trim().to_string();
    let peer = session
        .resolve_send_peer(&pk)
        .ok_or_else(|| "call media: unknown contact".to_string())?;
    if session.call_media_active(&call_id) {
        crate::p2p::call_active::on_voice_start(&call_id, &pk);
        emit_call_media(&events_tx, &call_id, &pk, "voice_started", None);
        return Ok(());
    }
    let keys = derive_call_media_keys(session.as_ref(), &pk, &call_id)?;
    let local_is_a = crate::call_media::local_is_a(&session.identity.identity_wire(), &pk);
    let engine = crate::call_media::MediaEngine::new_opus(&keys.frame_key, local_is_a)?;
    #[cfg(target_os = "android")]
    let _ = crate::call_media::ensure_voice_audio_mode();
    let mut backend = crate::call_media::default_audio_backend();
    let audio = backend
        .start()
        .map_err(|e| format!("call media: audio start: {e}"))?;
    let controls = crate::call_media::MediaControls::new();
    let (wire_out_tx, mut wire_out_rx) = mpsc::channel::<Vec<u8>>(256);
    let (wire_in_tx, wire_in_rx) = mpsc::channel::<Vec<u8>>(256);
    session.call_media_register(call_id.clone(), peer.clone(), controls.clone(), wire_in_tx);
    let header = serde_json::to_vec(&CallStreamHeader {
        call_id: call_id.clone(),
    })
    .unwrap_or_default();
    let _ = queue_mux_for_peer(&writers, &peer, CHANNEL_CALL_AUDIO, header);
    let writers_tx = Arc::clone(&writers);
    let peer_tx = peer.clone();
    let ctl = controls.clone();
    tokio::spawn(async move {
        while let Some(bytes) = wire_out_rx.recv().await {
            if ctl.is_stopped() {
                break;
            }
            if queue_mux_for_peer(&writers_tx, &peer_tx, CHANNEL_CALL_AUDIO, bytes).is_err() {
                break;
            }
        }
        ctl.request_stop();
    });
    let session_ctl = controls.clone();
    tokio::spawn(async move {
        crate::call_media::run_media_session(engine, audio, wire_out_tx, wire_in_rx, session_ctl)
            .await;
        backend.stop();
    });
    crate::p2p::call_active::on_voice_start(&call_id, &pk);
    emit_call_media(&events_tx, &call_id, &pk, "voice_started", None);
    Ok(())
}

pub(crate) async fn start_call_video_for_peer(
    session: Arc<SessionState>,
    writers: SessionWriters,
    call_id: String,
    peer_public_key_hex: String,
    camera_enabled: bool,
    events_tx: Option<std::sync::mpsc::Sender<GossipChatEvent>>,
) -> Result<(), String> {
    use crate::call_video::{
        DEFAULT_REASSEMBLY_PENDING, DEFAULT_VIDEO_JITTER_MAX, RawVideoFrame, VideoControls,
        VideoEngine, VideoStreams, run_video_session,
    };
    let pk = peer_public_key_hex.trim().to_string();
    let peer = session
        .resolve_send_peer(&pk)
        .ok_or_else(|| "call video: unknown contact".to_string())?;
    if session.call_video_active(&call_id) {
        if camera_enabled {
            session.call_video_set_camera_off(&call_id, false);
            crate::p2p::call_active::set_camera_on(&call_id, true);
        }
        crate::p2p::call_active::on_video_start(&call_id, &pk, camera_enabled);
        return Ok(());
    }
    crate::call_video::track_call_shm(&call_id);
    let video_key_id = format!("{call_id}:video");
    let keys = derive_call_media_keys(session.as_ref(), &pk, &video_key_id)?;
    let local_is_a = crate::call_media::local_is_a(&session.identity.identity_wire(), &pk);
    let engine = VideoEngine::with_params(
        &keys.frame_key,
        local_is_a,
        Box::new(crate::call_video::H264Encoder::new()?),
        Box::new(crate::call_video::H264Decoder::new()?),
        16 * 1024,
        DEFAULT_REASSEMBLY_PENDING,
        DEFAULT_VIDEO_JITTER_MAX,
    );
    let controls = VideoControls::new();
    let (wire_out_tx, mut wire_out_rx) = mpsc::channel::<Vec<u8>>(256);
    let (wire_in_tx, wire_in_rx) = mpsc::channel::<Vec<u8>>(256);
    session.call_video_register(call_id.clone(), peer.clone(), controls.clone(), wire_in_tx);
    if camera_enabled {
        controls.set_camera_off(false);
    }
    let header = serde_json::to_vec(&CallStreamHeader {
        call_id: call_id.clone(),
    })
    .unwrap_or_default();
    let _ = super::peer_session::queue_mux_for_peer(&writers, &peer, CHANNEL_CALL_VIDEO, header);
    let writers_tx = Arc::clone(&writers);
    let peer_tx = peer.clone();
    let ctl = controls.clone();
    tokio::spawn(async move {
        while let Some(bytes) = wire_out_rx.recv().await {
            if ctl.is_stopped() {
                break;
            }
            if super::peer_session::queue_mux_for_peer(
                &writers_tx,
                &peer_tx,
                CHANNEL_CALL_VIDEO,
                bytes,
            )
            .is_err()
            {
                break;
            }
        }
        ctl.request_stop();
    });
    let capture_rx = match crate::call_video::spawn_camera_capture(controls.clone()) {
        Ok(rx) => rx,
        Err(e) => {
            native_log::warn("call_video", format!("camera unavailable: {e}"));
            let (keep_tx, rx) = mpsc::channel::<RawVideoFrame>(1);
            let ctl = controls.clone();
            tokio::spawn(async move {
                while !ctl.is_stopped() {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
                drop(keep_tx);
            });
            rx
        }
    };
    let (render_tx, mut render_rx) = mpsc::channel::<RawVideoFrame>(8);
    let render_call_id = call_id.clone();
    tokio::spawn(async move {
        while let Some(frame) = render_rx.recv().await {
            crate::call_video::publish_decoded_frame(&render_call_id, frame);
        }
    });
    let streams = VideoStreams {
        capture_rx,
        render_tx,
    };
    let session_ctl = controls.clone();
    let session_call_id = call_id.clone();
    tokio::spawn(async move {
        run_video_session(
            engine,
            streams,
            session_call_id,
            wire_out_tx,
            wire_in_rx,
            session_ctl,
        )
        .await;
    });
    crate::p2p::call_active::on_video_start(&call_id, &pk, camera_enabled);
    emit_call_media(&events_tx, &call_id, &pk, "video_started", None);
    Ok(())
}

fn derive_call_media_keys(
    session: &SessionState,
    peer_identity_wire: &str,
    call_id: &str,
) -> Result<crate::call_media_key::CallMediaKeys, String> {
    let peer_wire = crate::public_key_util::normalize_contact_identity_wire(peer_identity_wire)?;
    let peer_transport_pk = session
        .peer_transport_pk(&peer_wire)
        .ok_or_else(|| "call media: transport kem not ready for peer".to_string())?;
    crate::call_media_key::derive_call_media_keys_from_transport(
        session.dm_local_transport_sk(),
        &peer_transport_pk,
        &session.identity.identity_wire(),
        &peer_wire,
        call_id,
    )
}
