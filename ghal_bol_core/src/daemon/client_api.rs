//! Canonical **daemon ↔ UI integrator** contract.
//!
//! The background node (`ghal_bol_core_daemon` / Android `:p2p`) owns product behaviour.
//! Host UI shells must use only the RPC methods and wake signals declared here.
//! Integrator architecture: `docs/DAEMON_INTEGRATOR.md`.
//!
//! Wire format: newline-delimited JSON `{ "id", "method", "params" }` on the Unix socket.
//! Method names are stable string literals from [`DaemonMethod::wire_name`].

use serde_json::Value;

use crate::coord_runtime;
use crate::p2p_runtime;
use crate::session_runtime;

/// JSON-RPC method names the daemon accepts from an integrator UI.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DaemonMethod {
    Ping,
    Unlock,
    Lock,
    SessionUnlocked,
    P2pStart,
    P2pStop,
    P2pIsRunning,
    NetworkSnapshot,
    P2pPoll,
    P2pSendTextDm,
    P2pSendVoiceDm,
    P2pSendAttachment,
    P2pAttachmentFetch,
    P2pAttachmentCancel,
    P2pCallSignal,
    P2pCallMedia,
    P2pCallStatus,
    P2pDismissIncomingCallAlert,
    P2pForceEndActiveCall,
    P2pTakeIncomingCallWake,
    P2pTakeUnlockWake,
    UiSessionPrepareReconnect,
    UiProcessExiting,
    P2pTranscriptLoadMerged,
    P2pCallVideo,
    P2pCallVideoFrame,
    P2pCallVideoTexture,
    P2pCallVideoPushCameraFrame,
    P2pRequeueOutboundDm,
    P2pSendAckDm,
    P2pDialBootstrap,
    P2pRegisterDmPeer,
    P2pSetAvailabilityStatus,
    P2pGetAvailabilityStatus,
    /// Deprecated — use [`DaemonMethod::P2pSyncUiSession`].
    P2pSetAppAckReadEnabled,
    /// Deprecated — use [`DaemonMethod::P2pSyncUiSession`].
    P2pSetAppUiVisible,
    P2pSyncUiSession,
    P2pNudgeReadCatchup,
    /// Deprecated — use [`DaemonMethod::P2pSyncUiSession`].
    P2pSetForegroundPeer,
    CoordSetBaseUrl,
    CoordLookupPeer,
    CoordRegisterNow,
    DeliveryConnectionStatus,
    DeliveryQuotaStatus,
    DeliveryMailboxList,
    DeliveryExtendTtl,
    DeliveryResendMessage,
}

impl DaemonMethod {
    /// Stable wire name (JSON `"method"` field).
    pub const fn wire_name(self) -> &'static str {
        match self {
            Self::Ping => "ping",
            Self::Unlock => "unlock",
            Self::Lock => "lock",
            Self::SessionUnlocked => "session_unlocked",
            Self::P2pStart => "p2p_start",
            Self::P2pStop => "p2p_stop",
            Self::P2pIsRunning => "p2p_is_running",
            Self::NetworkSnapshot => "network_snapshot",
            Self::P2pPoll => "p2p_poll",
            Self::P2pSendTextDm => "p2p_send_text_dm",
            Self::P2pSendVoiceDm => "p2p_send_voice_dm",
            Self::P2pSendAttachment => "p2p_send_attachment",
            Self::P2pAttachmentFetch => "p2p_attachment_fetch",
            Self::P2pAttachmentCancel => "p2p_attachment_cancel",
            Self::P2pCallSignal => "p2p_call_signal",
            Self::P2pCallMedia => "p2p_call_media",
            Self::P2pCallStatus => "p2p_call_status",
            Self::P2pDismissIncomingCallAlert => "p2p_dismiss_incoming_call_alert",
            Self::P2pForceEndActiveCall => "p2p_force_end_active_call",
            Self::P2pTakeIncomingCallWake => "p2p_take_incoming_call_wake",
            Self::P2pTakeUnlockWake => "p2p_take_unlock_wake",
            Self::UiSessionPrepareReconnect => "ui_session_prepare_reconnect",
            Self::UiProcessExiting => "ui_process_exiting",
            Self::P2pTranscriptLoadMerged => "p2p_transcript_load_merged",
            Self::P2pCallVideo => "p2p_call_video",
            Self::P2pCallVideoFrame => "p2p_call_video_frame",
            Self::P2pCallVideoTexture => "p2p_call_video_texture",
            Self::P2pCallVideoPushCameraFrame => "p2p_call_video_push_camera_frame",
            Self::P2pRequeueOutboundDm => "p2p_requeue_outbound_dm",
            Self::P2pSendAckDm => "p2p_send_ack_dm",
            Self::P2pDialBootstrap => "p2p_dial_bootstrap",
            Self::P2pRegisterDmPeer => "p2p_register_dm_peer",
            Self::P2pSetAvailabilityStatus => "p2p_set_availability_status",
            Self::P2pGetAvailabilityStatus => "p2p_get_availability_status",
            Self::P2pSetAppAckReadEnabled => "p2p_set_app_ack_read_enabled",
            Self::P2pSetAppUiVisible => "p2p_set_app_ui_visible",
            Self::P2pSyncUiSession => "p2p_sync_ui_session",
            Self::P2pNudgeReadCatchup => "p2p_nudge_read_catchup",
            Self::P2pSetForegroundPeer => "p2p_set_foreground_peer",
            Self::CoordSetBaseUrl => "coord_set_base_url",
            Self::CoordLookupPeer => "coord_lookup_peer",
            Self::CoordRegisterNow => "coord_register_now",
            Self::DeliveryConnectionStatus => "delivery_connection_status",
            Self::DeliveryQuotaStatus => "delivery_quota_status",
            Self::DeliveryMailboxList => "delivery_mailbox_list",
            Self::DeliveryExtendTtl => "delivery_extend_ttl",
            Self::DeliveryResendMessage => "delivery_resend_message",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "ping" => Self::Ping,
            "unlock" => Self::Unlock,
            "lock" => Self::Lock,
            "session_unlocked" => Self::SessionUnlocked,
            "p2p_start" => Self::P2pStart,
            "p2p_stop" => Self::P2pStop,
            "p2p_is_running" => Self::P2pIsRunning,
            "network_snapshot" => Self::NetworkSnapshot,
            "p2p_poll" => Self::P2pPoll,
            "p2p_send_text_dm" => Self::P2pSendTextDm,
            "p2p_send_voice_dm" => Self::P2pSendVoiceDm,
            "p2p_send_attachment" => Self::P2pSendAttachment,
            "p2p_attachment_fetch" => Self::P2pAttachmentFetch,
            "p2p_attachment_cancel" => Self::P2pAttachmentCancel,
            "p2p_call_signal" => Self::P2pCallSignal,
            "p2p_call_media" => Self::P2pCallMedia,
            "p2p_call_status" => Self::P2pCallStatus,
            "p2p_dismiss_incoming_call_alert" => Self::P2pDismissIncomingCallAlert,
            "p2p_force_end_active_call" => Self::P2pForceEndActiveCall,
            "p2p_take_incoming_call_wake" => Self::P2pTakeIncomingCallWake,
            "p2p_take_unlock_wake" => Self::P2pTakeUnlockWake,
            "ui_session_prepare_reconnect" => Self::UiSessionPrepareReconnect,
            "ui_process_exiting" => Self::UiProcessExiting,
            "p2p_transcript_load_merged" => Self::P2pTranscriptLoadMerged,
            "p2p_call_video" => Self::P2pCallVideo,
            "p2p_call_video_frame" => Self::P2pCallVideoFrame,
            "p2p_call_video_texture" => Self::P2pCallVideoTexture,
            "p2p_call_video_push_camera_frame" => Self::P2pCallVideoPushCameraFrame,
            "p2p_requeue_outbound_dm" => Self::P2pRequeueOutboundDm,
            "p2p_send_ack_dm" => Self::P2pSendAckDm,
            "p2p_dial_bootstrap" => Self::P2pDialBootstrap,
            "p2p_register_dm_peer" => Self::P2pRegisterDmPeer,
            "p2p_set_availability_status" => Self::P2pSetAvailabilityStatus,
            "p2p_get_availability_status" => Self::P2pGetAvailabilityStatus,
            "p2p_set_app_ack_read_enabled" => Self::P2pSetAppAckReadEnabled,
            "p2p_set_app_ui_visible" => Self::P2pSetAppUiVisible,
            "p2p_sync_ui_session" => Self::P2pSyncUiSession,
            "p2p_nudge_read_catchup" => Self::P2pNudgeReadCatchup,
            "p2p_set_foreground_peer" => Self::P2pSetForegroundPeer,
            "coord_set_base_url" => Self::CoordSetBaseUrl,
            "coord_lookup_peer" => Self::CoordLookupPeer,
            "coord_register_now" => Self::CoordRegisterNow,
            "delivery_connection_status" => Self::DeliveryConnectionStatus,
            "delivery_quota_status" => Self::DeliveryQuotaStatus,
            "delivery_mailbox_list" => Self::DeliveryMailboxList,
            "delivery_extend_ttl" => Self::DeliveryExtendTtl,
            "delivery_resend_message" => Self::DeliveryResendMessage,
            _ => return None,
        })
    }

    /// Every method handled by [`dispatch_method`] — keep in sync with tests.
    pub const ALL: &'static [Self] = &[
        Self::Ping,
        Self::Unlock,
        Self::Lock,
        Self::SessionUnlocked,
        Self::P2pStart,
        Self::P2pStop,
        Self::P2pIsRunning,
        Self::NetworkSnapshot,
        Self::P2pPoll,
        Self::P2pSendTextDm,
        Self::P2pSendVoiceDm,
        Self::P2pSendAttachment,
        Self::P2pAttachmentFetch,
        Self::P2pAttachmentCancel,
        Self::P2pCallSignal,
        Self::P2pCallMedia,
        Self::P2pCallStatus,
        Self::P2pDismissIncomingCallAlert,
        Self::P2pForceEndActiveCall,
        Self::P2pTakeIncomingCallWake,
        Self::P2pTakeUnlockWake,
        Self::UiSessionPrepareReconnect,
        Self::UiProcessExiting,
        Self::P2pTranscriptLoadMerged,
        Self::P2pCallVideo,
        Self::P2pCallVideoFrame,
        Self::P2pCallVideoTexture,
        Self::P2pCallVideoPushCameraFrame,
        Self::P2pRequeueOutboundDm,
        Self::P2pSendAckDm,
        Self::P2pDialBootstrap,
        Self::P2pRegisterDmPeer,
        Self::P2pSetAvailabilityStatus,
        Self::P2pGetAvailabilityStatus,
        Self::P2pSetAppAckReadEnabled,
        Self::P2pSetAppUiVisible,
        Self::P2pSyncUiSession,
        Self::P2pNudgeReadCatchup,
        Self::P2pSetForegroundPeer,
        Self::CoordSetBaseUrl,
        Self::CoordLookupPeer,
        Self::CoordRegisterNow,
        Self::DeliveryConnectionStatus,
        Self::DeliveryQuotaStatus,
        Self::DeliveryMailboxList,
        Self::DeliveryExtendTtl,
        Self::DeliveryResendMessage,
    ];
}

/// Daemon-initiated signals for the integrator UI (runtime files + OS notifications).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiWakeKind {
    /// Keystore exists but session locked — present password UI (Linux autostart).
    Unlock,
    /// Incoming call invite — present call UI.
    IncomingCall,
}

impl UiWakeKind {
    pub const fn runtime_marker_file(self) -> &'static str {
        match self {
            Self::Unlock => "unlock_wake",
            Self::IncomingCall => "incoming_call_wake",
        }
    }

    /// Integrator presence marker (UI process touched on shell startup).
    pub const UI_PRESENCE_FILE: &'static str = "ui_present";
}

/// Common `p2p_poll` event `kind` values (payload is still JSON — see wire logs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DaemonPollEventKind {
    NodeReady,
    NodeStopped,
    PeerConnected,
    PeerDisconnected,
    DmMessage,
    StoresUpdated,
    PeerIdentified,
    Listening,
    ChatReady,
}

impl DaemonPollEventKind {
    pub const fn wire_kind(self) -> &'static str {
        match self {
            Self::NodeReady => "node_ready",
            Self::NodeStopped => "node_stopped",
            Self::PeerConnected => "peer_connected",
            Self::PeerDisconnected => "peer_disconnected",
            Self::DmMessage => "dm_message",
            Self::StoresUpdated => "stores_updated",
            Self::PeerIdentified => "peer_identified",
            Self::Listening => "listening",
            Self::ChatReady => "chat_ready",
        }
    }
}

/// Host UI responsibilities (implemented in Flutter/platform — documented contract).
///
/// The daemon drives **when** to wake; the integrator implements **how** to present.
pub trait UiIntegratorCallbacks {
    /// Raise the main window (GTK `present`, Android activity, etc.).
    fn present_main_window(&self);
    /// Consume a daemon wake marker and map to UI navigation.
    fn on_daemon_wake(&self, kind: UiWakeKind);
    /// Push lifecycle + open room via a single `p2p_sync_ui_session` snapshot.
    fn sync_ui_session(&self, ui_visible: bool, room_public_key_hex: Option<&str>);
}

/// Dispatch a parsed RPC method. Returns JSON result or error string.
pub fn dispatch_method(method: DaemonMethod, params: &Value) -> Result<Value, String> {
    match method {
        DaemonMethod::Ping => Ok(serde_json::json!({ "ok": true, "pong": true })),
        DaemonMethod::Unlock => {
            let ns = param_str(params, "app_namespace")?;
            let password = param_str(params, "password")?;
            let result = session_runtime::unlock_identity(&ns, &password);
            if result.get("ok").and_then(|v| v.as_bool()) == Some(true) {
                crate::daemon::clear_unlock_wake();
            }
            Ok(result)
        }
        DaemonMethod::Lock => {
            session_runtime::lock_identity();
            Ok(serde_json::json!({ "ok": true }))
        }
        DaemonMethod::SessionUnlocked => Ok(serde_json::json!({
            "ok": true,
            "unlocked": session_runtime::session_unlocked(),
        })),
        DaemonMethod::P2pStart => {
            let config = params
                .get("config")
                .cloned()
                .unwrap_or_else(|| params.clone());
            Ok(p2p_runtime::p2p_start(&config))
        }
        DaemonMethod::P2pStop => {
            p2p_runtime::p2p_stop();
            Ok(serde_json::json!({ "ok": true }))
        }
        DaemonMethod::P2pIsRunning => Ok(p2p_runtime::p2p_is_running()),
        DaemonMethod::NetworkSnapshot => Ok(crate::network_ffi::network_snapshot_rpc()),
        DaemonMethod::P2pPoll => Ok(match p2p_runtime::p2p_poll_event() {
            Some(ev) => serde_json::json!({ "ok": true, "event": ev }),
            None => serde_json::json!({ "ok": true, "event": null }),
        }),
        DaemonMethod::P2pSendTextDm => {
            let recipient = param_str(params, "recipient_public_key_hex")?;
            let text = param_str(params, "text")?;
            Ok(p2p_runtime::p2p_send_text_dm(&recipient, &text))
        }
        DaemonMethod::P2pSendVoiceDm => {
            let recipient = param_str(params, "recipient_public_key_hex")?;
            Ok(p2p_runtime::p2p_send_voice_dm_from_config(
                &recipient, params,
            ))
        }
        DaemonMethod::P2pSendAttachment => {
            let recipient = param_str(params, "recipient_public_key_hex")?;
            Ok(p2p_runtime::p2p_send_attachment(&recipient, params))
        }
        DaemonMethod::P2pAttachmentFetch => Ok(p2p_runtime::p2p_attachment_fetch(params)),
        DaemonMethod::P2pAttachmentCancel => Ok(p2p_runtime::p2p_attachment_cancel(params)),
        DaemonMethod::P2pCallSignal => Ok(p2p_runtime::p2p_call_signal(params)),
        DaemonMethod::P2pCallMedia => Ok(p2p_runtime::p2p_call_media(params)),
        DaemonMethod::P2pCallStatus => Ok(p2p_runtime::p2p_call_status(params)),
        DaemonMethod::P2pDismissIncomingCallAlert => {
            Ok(p2p_runtime::p2p_dismiss_incoming_call_alert())
        }
        DaemonMethod::P2pForceEndActiveCall => {
            let reason = params
                .get("reason")
                .and_then(|v| v.as_str())
                .unwrap_or("rpc");
            Ok(p2p_runtime::p2p_force_end_active_call(reason))
        }
        DaemonMethod::P2pTakeIncomingCallWake => Ok(p2p_runtime::p2p_take_incoming_call_wake()),
        DaemonMethod::P2pTakeUnlockWake => Ok(p2p_runtime::p2p_take_unlock_wake()),
        DaemonMethod::UiSessionPrepareReconnect => {
            let ms = params
                .get("suppress_ms")
                .and_then(|v| v.as_i64())
                .unwrap_or(5_000);
            crate::daemon::suppress_ui_exit_hangup_ms(ms);
            Ok(serde_json::json!({ "ok": true }))
        }
        DaemonMethod::UiProcessExiting => {
            crate::daemon::ui_process_exiting();
            Ok(serde_json::json!({ "ok": true }))
        }
        DaemonMethod::P2pTranscriptLoadMerged => {
            Ok(p2p_runtime::p2p_transcript_load_merged(params))
        }
        DaemonMethod::P2pCallVideo => Ok(p2p_runtime::p2p_call_video(params)),
        DaemonMethod::P2pCallVideoFrame => Ok(p2p_runtime::p2p_call_video_frame(params)),
        DaemonMethod::P2pCallVideoTexture => Ok(p2p_runtime::p2p_call_video_texture(params)),
        DaemonMethod::P2pCallVideoPushCameraFrame => {
            Ok(p2p_runtime::p2p_call_video_push_camera_frame(params))
        }
        DaemonMethod::P2pRequeueOutboundDm => {
            let message_id = param_str(params, "message_id")?;
            let recipient = param_str(params, "recipient_public_key_hex")?;
            let text = param_str(params, "text")?;
            Ok(p2p_runtime::p2p_requeue_outbound_dm(
                &message_id,
                &recipient,
                &text,
            ))
        }
        DaemonMethod::P2pSendAckDm => {
            let recipient = param_str(params, "recipient_public_key_hex")?;
            let ref_id = param_str(params, "ref_id")?;
            let ack_kind = param_str(params, "ack_kind")?;
            Ok(p2p_runtime::p2p_send_ack_dm(&recipient, &ref_id, &ack_kind))
        }
        DaemonMethod::P2pDialBootstrap => {
            let addrs: Vec<String> = params
                .get("bootstrap_peers")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|x| x.as_str())
                        .map(|s| s.to_string())
                        .collect()
                })
                .unwrap_or_default();
            let parsed: Vec<crate::dm_transport::DmDialAddr> = addrs
                .iter()
                .filter_map(|s| crate::dm_transport::DmDialAddr::parse(s))
                .collect();
            Ok(p2p_runtime::p2p_dial_bootstrap_peers(&parsed))
        }
        DaemonMethod::P2pRegisterDmPeer => {
            let pk = param_str(params, "public_key_hex")?;
            Ok(p2p_runtime::p2p_register_dm_peer(&pk))
        }
        DaemonMethod::P2pSetAvailabilityStatus => {
            let status = params.get("status").and_then(|v| v.as_str()).unwrap_or("");
            Ok(p2p_runtime::p2p_set_availability_status(status))
        }
        DaemonMethod::P2pGetAvailabilityStatus => Ok(p2p_runtime::p2p_get_availability_status()),
        DaemonMethod::P2pSetAppAckReadEnabled => {
            let enabled = params
                .get("enabled")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            Ok(p2p_runtime::p2p_set_app_ack_read_enabled(enabled))
        }
        DaemonMethod::P2pSetAppUiVisible => {
            let visible = params
                .get("visible")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            Ok(p2p_runtime::p2p_set_app_ui_visible(visible))
        }
        DaemonMethod::P2pSyncUiSession => {
            let ui_visible = params
                .get("ui_visible")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let room = params
                .get("room_public_key_hex")
                .or_else(|| params.get("public_key_hex"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty());
            Ok(p2p_runtime::p2p_sync_ui_session(ui_visible, room))
        }
        DaemonMethod::P2pNudgeReadCatchup => Ok(p2p_runtime::p2p_nudge_read_catchup()),
        DaemonMethod::P2pSetForegroundPeer => {
            let pk = params
                .get("public_key_hex")
                .or_else(|| params.get("peer_id"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty());
            Ok(p2p_runtime::p2p_set_foreground_peer(pk))
        }
        DaemonMethod::CoordSetBaseUrl => {
            let insecure = params
                .get("insecure_tls")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let urls = coord_runtime::coord_urls_from_json_value(params);
            if urls.is_empty() {
                return Err("base_url or base_urls required".to_string());
            }
            Ok(coord_runtime::coord_set_base_urls_json(&urls, insecure))
        }
        DaemonMethod::CoordLookupPeer => {
            let pk = param_str(params, "public_key_hex")?;
            Ok(coord_runtime::coord_lookup_peer_json(&pk))
        }
        DaemonMethod::CoordRegisterNow => Ok(coord_runtime::coord_register_now_json()),
        DaemonMethod::DeliveryConnectionStatus => {
            Ok(crate::delivery_runtime::delivery_connection_status())
        }
        DaemonMethod::DeliveryQuotaStatus => Ok(crate::delivery_runtime::delivery_quota_status()),
        DaemonMethod::DeliveryMailboxList => {
            let include_expired = params
                .get("include_expired")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            Ok(crate::delivery_runtime::delivery_mailbox_list(
                include_expired,
            ))
        }
        DaemonMethod::DeliveryExtendTtl => {
            let message_id = param_str(params, "message_id")?;
            let extend_secs = params
                .get("extend_secs")
                .and_then(|v| v.as_u64())
                .ok_or_else(|| "missing extend_secs".to_string())?;
            Ok(crate::delivery_runtime::delivery_extend_ttl(
                &message_id,
                extend_secs,
            ))
        }
        DaemonMethod::DeliveryResendMessage => {
            let message_id = param_str(params, "message_id")?;
            Ok(crate::delivery_runtime::delivery_resend_message(
                &message_id,
            ))
        }
    }
}

fn param_str(params: &Value, key: &str) -> Result<String, String> {
    params
        .get(key)
        .and_then(|v| v.as_str())
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("missing param: {key}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_name_parse_roundtrip() {
        for method in DaemonMethod::ALL {
            let name = method.wire_name();
            assert_eq!(DaemonMethod::parse(name), Some(*method));
        }
    }

    #[test]
    fn all_methods_unique_wire_names() {
        use std::collections::HashSet;
        let mut seen = HashSet::new();
        for method in DaemonMethod::ALL {
            assert!(
                seen.insert(method.wire_name()),
                "duplicate {}",
                method.wire_name()
            );
        }
        assert_eq!(seen.len(), DaemonMethod::ALL.len());
    }

    #[test]
    fn ping_and_session_unlocked_dispatch() {
        let ping = dispatch_method(DaemonMethod::Ping, &Value::Null).unwrap();
        assert_eq!(ping["pong"], true);
        let unlocked = dispatch_method(DaemonMethod::SessionUnlocked, &Value::Null).unwrap();
        assert_eq!(unlocked["ok"], true);
    }

    #[test]
    fn all_method_count_matches_dart_mirror() {
        /// Keep in sync with `ghal_bol_ui/lib/daemon_client_api.dart` `DaemonMethod.all`.
        const DART_MIRROR_METHOD_COUNT: usize = 47;
        assert_eq!(DaemonMethod::ALL.len(), DART_MIRROR_METHOD_COUNT);
    }

    #[test]
    fn unknown_wire_name_not_in_all() {
        assert!(DaemonMethod::parse("not_a_real_method").is_none());
    }
}
