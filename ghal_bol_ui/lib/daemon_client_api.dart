/// Canonical **daemon ↔ UI integrator** wire names (mirror of `ghal_bol_core::daemon::client_api`).
///
/// Host UI must use these constants for JSON-RPC `"method"` fields — do not invent new
/// method strings outside this file. Product behaviour lives in Rust (`ghal_bol_core_daemon` / `:p2p`).
abstract final class DaemonMethod {
  static const ping = "ping";
  static const unlock = "unlock";
  static const lock = "lock";
  static const sessionUnlocked = "session_unlocked";
  static const p2pStart = "p2p_start";
  static const p2pStop = "p2p_stop";
  static const p2pIsRunning = "p2p_is_running";
  static const networkSnapshot = "network_snapshot";
  static const p2pPoll = "p2p_poll";
  static const p2pSendTextDm = "p2p_send_text_dm";
  static const p2pSendVoiceDm = "p2p_send_voice_dm";
  static const p2pSendAttachment = "p2p_send_attachment";
  static const p2pAttachmentFetch = "p2p_attachment_fetch";
  static const p2pAttachmentCancel = "p2p_attachment_cancel";
  static const p2pCallSignal = "p2p_call_signal";
  static const p2pCallMedia = "p2p_call_media";
  static const p2pCallStatus = "p2p_call_status";
  static const p2pDismissIncomingCallAlert = "p2p_dismiss_incoming_call_alert";
  static const p2pForceEndActiveCall = "p2p_force_end_active_call";
  static const p2pTakeIncomingCallWake = "p2p_take_incoming_call_wake";
  static const p2pTakeUnlockWake = "p2p_take_unlock_wake";
  static const uiSessionPrepareReconnect = "ui_session_prepare_reconnect";
  static const uiProcessExiting = "ui_process_exiting";
  static const p2pTranscriptLoadMerged = "p2p_transcript_load_merged";
  static const p2pCallVideo = "p2p_call_video";
  static const p2pCallVideoFrame = "p2p_call_video_frame";
  static const p2pCallVideoTexture = "p2p_call_video_texture";
  static const p2pCallVideoPushCameraFrame = "p2p_call_video_push_camera_frame";
  static const p2pRequeueOutboundDm = "p2p_requeue_outbound_dm";
  static const p2pSendAckDm = "p2p_send_ack_dm";
  static const p2pDialBootstrap = "p2p_dial_bootstrap";
  static const p2pRegisterDmPeer = "p2p_register_dm_peer";
  static const p2pSetAvailabilityStatus = "p2p_set_availability_status";
  static const p2pGetAvailabilityStatus = "p2p_get_availability_status";
  static const p2pSetAppAckReadEnabled = "p2p_set_app_ack_read_enabled";
  static const p2pSetAppUiVisible = "p2p_set_app_ui_visible";
  static const p2pSyncUiSession = "p2p_sync_ui_session";
  static const p2pNudgeReadCatchup = "p2p_nudge_read_catchup";
  static const p2pSetForegroundPeer = "p2p_set_foreground_peer";
  static const coordSetBaseUrl = "coord_set_base_url";
  static const coordLookupPeer = "coord_lookup_peer";
  static const coordRegisterNow = "coord_register_now";
  static const deliveryConnectionStatus = "delivery_connection_status";
  static const deliveryQuotaStatus = "delivery_quota_status";
  static const deliveryMailboxList = "delivery_mailbox_list";
  static const deliveryExtendTtl = "delivery_extend_ttl";
  static const deliveryResendMessage = "delivery_resend_message";

  /// Keep in sync with `DaemonMethod::ALL` in `ghal_bol_core/src/daemon/client_api.rs`.
  /// Parity: `./scripts/check_daemon_sdk_parity.sh`.
  static const all = <String>[
    ping,
    unlock,
    lock,
    sessionUnlocked,
    p2pStart,
    p2pStop,
    p2pIsRunning,
    networkSnapshot,
    p2pPoll,
    p2pSendTextDm,
    p2pSendVoiceDm,
    p2pSendAttachment,
    p2pAttachmentFetch,
    p2pAttachmentCancel,
    p2pCallSignal,
    p2pCallMedia,
    p2pCallStatus,
    p2pDismissIncomingCallAlert,
    p2pForceEndActiveCall,
    p2pTakeIncomingCallWake,
    p2pTakeUnlockWake,
    uiSessionPrepareReconnect,
    uiProcessExiting,
    p2pTranscriptLoadMerged,
    p2pCallVideo,
    p2pCallVideoFrame,
    p2pCallVideoTexture,
    p2pCallVideoPushCameraFrame,
    p2pRequeueOutboundDm,
    p2pSendAckDm,
    p2pDialBootstrap,
    p2pRegisterDmPeer,
    p2pSetAvailabilityStatus,
    p2pGetAvailabilityStatus,
    p2pSetAppAckReadEnabled,
    p2pSetAppUiVisible,
    p2pSyncUiSession,
    p2pNudgeReadCatchup,
    p2pSetForegroundPeer,
    coordSetBaseUrl,
    coordLookupPeer,
    coordRegisterNow,
    deliveryConnectionStatus,
    deliveryQuotaStatus,
    deliveryMailboxList,
    deliveryExtendTtl,
    deliveryResendMessage,
  ];
}

/// Daemon-initiated wake markers under `$XDG_RUNTIME_DIR/ghal_bol/`.
abstract final class UiWakeKind {
  static const unlockMarker = "unlock_wake";
  static const incomingCallMarker = "incoming_call_wake";
  static const uiPresenceMarker = "ui_present";
}

/// Common `p2p_poll` event `kind` values (payload remains JSON from native).
abstract final class DaemonPollEventKind {
  static const nodeReady = "node_ready";
  static const nodeStopped = "node_stopped";
  static const peerConnected = "peer_connected";
  static const peerDisconnected = "peer_disconnected";
  static const dmMessage = "dm_message";
  static const storesUpdated = "stores_updated";
  static const peerIdentified = "peer_identified";
  static const listening = "listening";
  static const chatReady = "chat_ready";
}

/// Integrator path helpers — must match `ghal_bol_core/src/daemon/paths.rs`.
/// See `docs/DAEMON_INTEGRATOR.md`.
abstract final class DaemonIntegratorConfig {
  static String sanitizeAppNamespaceSegment(String appNamespace) {
    final trimmed = appNamespace.trim();
    if (trimmed.isEmpty) return "default";
    final buf = StringBuffer();
    for (final c in trimmed.runes) {
      final ch = String.fromCharCode(c);
      if (_isSafeNamespaceChar(ch)) {
        buf.write(ch);
      } else {
        buf.write("_");
      }
    }
    return buf.toString();
  }

  static bool _isSafeNamespaceChar(String ch) {
    if (ch.length != 1) return false;
    final c = ch.codeUnitAt(0);
    final isAlphaNum =
        (c >= 0x30 && c <= 0x39) ||
        (c >= 0x41 && c <= 0x5a) ||
        (c >= 0x61 && c <= 0x7a);
    return isAlphaNum || ch == "." || ch == "-" || ch == "_";
  }

  /// `$XDG_RUNTIME_DIR/ghal_bol/<namespace>/` (or `/tmp/ghal_bol/...` when [xdgRuntimeDir] null).
  static String runtimeDirForAppNamespace(
    String appNamespace, {
    String? xdgRuntimeDir,
  }) {
    final runtime = xdgRuntimeDir?.trim();
    final base = runtime != null && runtime.isNotEmpty
        ? "$runtime/ghal_bol"
        : "/tmp/ghal_bol";
    return "$base/${sanitizeAppNamespaceSegment(appNamespace)}";
  }

  /// Default socket when `GHAL_BOL_DAEMON_SOCKET` is unset.
  static String socketPathForAppNamespace(
    String appNamespace, {
    String? xdgRuntimeDir,
  }) =>
      "${runtimeDirForAppNamespace(appNamespace, xdgRuntimeDir: xdgRuntimeDir)}/p2p.sock";

  static String uiPresencePathForAppNamespace(
    String appNamespace, {
    String? xdgRuntimeDir,
  }) =>
      "${runtimeDirForAppNamespace(appNamespace, xdgRuntimeDir: xdgRuntimeDir)}/${UiWakeKind.uiPresenceMarker}";
}
