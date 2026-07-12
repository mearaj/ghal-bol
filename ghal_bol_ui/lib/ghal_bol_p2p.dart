import "dart:io" show Platform;

import "package:ghal_bol_ui/daemon_client_api.dart";
import "package:ghal_bol_ui/user_flow_log.dart";
import "package:ghal_bol_ui/ghal_bol_ffi.dart";
import "package:ghal_bol_ui/p2p_event_log.dart";
import "package:ghal_bol_ui/public_key_hex.dart";
import "package:ghal_bol_ui/session_credentials.dart";
import "package:ghal_bol_ui/src/ghal_bol_daemon_client_io.dart"
    if (dart.library.html) "package:ghal_bol_ui/src/ghal_bol_daemon_client_stub.dart";

/// P2P transport: in-process FFI (desktop without daemon), or out-of-process daemon.
/// Set by [P2pEventBridge.ensureStarted] so [pollEventMap] and [waitNodeReady] share state.
typedef GhalBolPollEventDispatcher = void Function(Map<String, dynamic> ev);

abstract final class GhalBolP2p {
  static GhalBolPollEventDispatcher? pollEventDispatcher;

  static bool get usesDaemon => Platform.isLinux || Platform.isAndroid;

  static bool get isAvailable =>
      usesDaemon || GhalBolFfi.isP2pAvailable;

  static bool get isRequeueAvailable =>
      usesDaemon || GhalBolFfi.isP2pRequeueAvailable;

  static Future<Map<String, dynamic>> startJson(Map<String, dynamic> config) async {
    if (usesDaemon) {
      await GhalBolDaemonClient.ensureDaemonRunning();
      return GhalBolDaemonClient.instance.call(
        DaemonMethod.p2pStart,
        params: {"config": config},
      );
    }
    return GhalBolFfi.p2pStartJson(config);
  }

  static Future<void> stop() async {
    if (usesDaemon) {
      await GhalBolDaemonClient.instance.call(DaemonMethod.p2pStop);
      return;
    }
    GhalBolFfi.p2pStop();
  }

  static Future<bool> isRunning() async {
    if (usesDaemon) {
      if (!await GhalBolDaemonClient.probeDaemon()) return false;
      final r = await GhalBolDaemonClient.instance.call(
        DaemonMethod.p2pIsRunning,
        ensureDaemon: false,
      );
      return r["ok"] == true && r["running"] == true;
    }
    return GhalBolFfi.p2pIsRunning();
  }

  static Future<void> registerDmPeer(String publicKeyHex) async {
    if (usesDaemon) {
      await GhalBolDaemonClient.instance.call(
        DaemonMethod.p2pRegisterDmPeer,
        params: {"public_key_hex": publicKeyHex},
      );
      return;
    }
    GhalBolFfi.p2pRegisterDmPeer("", publicKeyHex);
  }

  static Future<Map<String, dynamic>> sendTextDm(
    String recipientPublicKeyHex,
    String text,
  ) async {
    if (usesDaemon) {
      // State socket — must not queue behind poll-driven p2p_register_dm_peer on main.
      return _callStateWithDaemonRecovery(
        DaemonMethod.p2pSendTextDm,
        params: {
          "recipient_public_key_hex": recipientPublicKeyHex,
          "text": text,
        },
      );
    }
    return GhalBolFfi.p2pSendTextDm(recipientPublicKeyHex, text);
  }

  static Future<Map<String, dynamic>> _callStateWithDaemonRecovery(
    String method, {
    required Map<String, dynamic> params,
  }) async {
    var r = await GhalBolDaemonClient.instance.callState(
      method,
      params: params,
      ensureDaemon: true,
    );
    if (r["ok"] == true) return r;
    final err = r["error"]?.toString() ?? "";
    if (!_isRecoverableDaemonRpcError(err)) return r;
    await GhalBolDaemonClient.reconnectDaemon();
    await SessionCredentials.ensureDaemonUnlocked();
    return GhalBolDaemonClient.instance.callState(
      method,
      params: params,
      ensureDaemon: false,
    );
  }

  static bool _isRecoverableDaemonRpcError(String err) {
    final low = err.toLowerCase();
    return low.contains("broken pipe") ||
        low.contains("connection reset") ||
        low.contains("daemon disconnected") ||
        low.contains("connection refused") ||
        low.contains("not running");
  }

  static Future<Map<String, dynamic>> requeueOutboundDm({
    required String messageId,
    required String recipientPublicKeyHex,
    required String text,
  }) async {
    if (usesDaemon) {
      return GhalBolDaemonClient.instance.callState(
        DaemonMethod.p2pRequeueOutboundDm,
        params: {
          "message_id": messageId,
          "recipient_public_key_hex": recipientPublicKeyHex,
          "text": text,
        },
        ensureDaemon: true,
      );
    }
    return GhalBolFfi.p2pRequeueOutboundDm(
      messageId: messageId,
      recipientPublicKeyHex: recipientPublicKeyHex,
      text: text,
    );
  }

  /// Atomically sync UI visibility + open room → native read-receipt policy (DESIGN.md § UI session).
  static Future<Map<String, dynamic>> syncUiSession({
    required bool uiVisible,
    String? roomPublicKeyHex,
  }) async {
    final pk = resolvePublicKeyHex(storedHex: roomPublicKeyHex) ?? "";
    final params = <String, dynamic>{"ui_visible": uiVisible};
    if (isValidPublicKeyHex(pk)) {
      params["room_public_key_hex"] = pk;
    }
    if (usesDaemon) {
      return GhalBolDaemonClient.instance.callState(
        DaemonMethod.p2pSyncUiSession,
        params: params,
        ensureDaemon: true,
      );
    }
    return _inProcessSyncUiSession(uiVisible: uiVisible, pk: pk);
  }

  static Future<Map<String, dynamic>> _inProcessSyncUiSession({
    required bool uiVisible,
    required String pk,
  }) async {
    await _inProcessSetAppUiVisible(uiVisible);
    if (!isValidPublicKeyHex(pk)) {
      await _inProcessSetForegroundPeer(null);
      return _inProcessSetAppAckReadEnabled(false);
    }
    if (uiVisible) {
      await _inProcessSetAppAckReadEnabled(true);
      return _inProcessSetForegroundPeer(pk);
    }
    await _inProcessSetAppAckReadEnabled(false);
    return {"ok": true, "ui_visible": uiVisible, "read_receipts": false};
  }

  static Future<Map<String, dynamic>> _inProcessSetAppAckReadEnabled(bool enabled) async =>
      GhalBolFfi.p2pSetAppAckReadEnabled(enabled);

  static Future<Map<String, dynamic>> _inProcessSetAppUiVisible(bool visible) async =>
      GhalBolFfi.p2pSetAppUiVisible(visible);

  static Future<Map<String, dynamic>> _inProcessSetForegroundPeer(String? publicKeyHex) async =>
      GhalBolFfi.p2pSetForegroundPeer(publicKeyHex);

  /// Re-run in-room `ack_read` catch-up without re-issuing foreground room enter (Linux nudge).
  static Future<Map<String, dynamic>> nudgeReadCatchup() async {
    if (usesDaemon) {
      return GhalBolDaemonClient.instance.callState(
        DaemonMethod.p2pNudgeReadCatchup,
        ensureDaemon: true,
      );
    }
    return {"ok": true};
  }

  /// Voice/video call signaling (`invite`, `accept`, `sdp_offer`, `ice`, `video_on`, …).
  static Future<Map<String, dynamic>> callSignal(Map<String, dynamic> config) async {
    if (usesDaemon) {
      // State socket — must not queue behind `p2p_call_video_frame` polls on main (~60/s in-call).
      return GhalBolDaemonClient.instance.callState(
        DaemonMethod.p2pCallSignal,
        params: config,
      );
    }
    return GhalBolFfi.p2pCallSignal(config);
  }

  /// Native voice **media** control plane (Rust-owned Opus over libp2p substream).
  /// `action`: `start` (needs `recipient_public_key_hex`), `stop`, `set_mic_muted`.
  static Future<Map<String, dynamic>> callMedia(Map<String, dynamic> config) async {
    if (usesDaemon) {
      return GhalBolDaemonClient.instance.callState(
        DaemonMethod.p2pCallMedia,
        params: config,
      );
    }
    return GhalBolFfi.p2pCallMedia(config);
  }

  static Future<Map<String, dynamic>> callMediaStart({
    required String callId,
    required String recipientPublicKeyHex,
  }) {
    return callMedia({
      "action": "start",
      "call_id": callId,
      "recipient_public_key_hex": recipientPublicKeyHex,
    });
  }

  static Future<Map<String, dynamic>> callMediaStop({required String callId}) {
    return callMedia({"action": "stop", "call_id": callId});
  }

  static Future<Map<String, dynamic>> callMediaSetMicMuted({
    required String callId,
    required bool muted,
  }) {
    return callMedia({
      "action": "set_mic_muted",
      "call_id": callId,
      "muted": muted,
    });
  }

  static Future<Map<String, dynamic>> callMediaSetSpeaker({
    required String callId,
    required bool speakerOn,
  }) {
    return callMedia({
      "action": "set_speaker",
      "call_id": callId,
      "speaker_on": speakerOn,
    });
  }

  /// Active native voice/video session snapshot (`:p2p` may outlive the UI on Android).
  static Future<Map<String, dynamic>> callStatus() async {
    const config = <String, dynamic>{};
    if (usesDaemon) {
      return GhalBolDaemonClient.instance.callState(
        DaemonMethod.p2pCallStatus,
        params: config,
      );
    }
    return GhalBolFfi.p2pCallStatus(config);
  }

  /// Dismiss OS incoming-call alert owned by `:p2p` (Linux libnotify / Android full-screen).
  static Future<void> dismissIncomingCallAlert() async {
    if (usesDaemon) {
      await GhalBolDaemonClient.instance.callState(
        DaemonMethod.p2pDismissIncomingCallAlert,
        params: const {},
      );
      return;
    }
    await GhalBolFfi.p2pDismissIncomingCallAlert();
  }

  /// Privacy: stop native media and hang up when the UI session ends.
  static Future<Map<String, dynamic>> forceEndActiveCall({
    String reason = "ui_exit",
  }) async {
    final params = <String, dynamic>{"reason": reason};
    if (usesDaemon) {
      return GhalBolDaemonClient.instance.callState(
        DaemonMethod.p2pForceEndActiveCall,
        params: params,
      );
    }
    return GhalBolFfi.p2pForceEndActiveCall(params);
  }

  /// Linux daemon notification tap → present call UI (consumes wake marker).
  static Future<bool> takeIncomingCallWake() async {
    if (usesDaemon) {
      final r = await GhalBolDaemonClient.instance.callState(
        DaemonMethod.p2pTakeIncomingCallWake,
        params: const {},
      );
      return r["wake"] == true;
    }
    return GhalBolFfi.p2pTakeIncomingCallWake()["wake"] == true;
  }

  /// Linux daemon reboot unlock wake → present password UI (consumes wake marker).
  static Future<bool> takeUnlockWake() async {
    if (!usesDaemon) return false;
    final r = await GhalBolDaemonClient.instance.callState(
      DaemonMethod.p2pTakeUnlockWake,
      params: const {},
      ensureDaemon: false,
    );
    return r["wake"] == true;
  }

  /// Best-effort before UI socket reconnect (login unlock) — avoids hangup on transient EOF.
  static Future<void> suppressUiExitHangup({int suppressMs = 5000}) async {
    if (!usesDaemon) return;
    await GhalBolDaemonClient.instance.call(
      DaemonMethod.uiSessionPrepareReconnect,
      params: {"suppress_ms": suppressMs},
    );
  }

  /// Best-effort before process exit (Ctrl+C may skip this; daemon uses socket EOF).
  static Future<void> notifyUiProcessExiting() async {
    if (usesDaemon) {
      await GhalBolDaemonClient.instance.call(DaemonMethod.uiProcessExiting);
      return;
    }
    await forceEndActiveCall(reason: "ui_process_exiting");
  }

  /// Read-only transcript merge via background `:p2p` (same process that writes on poll).
  static Future<({int revision, List<Map<String, dynamic>> lines})> transcriptLoadThreadView({
    required String appNamespace,
    required List<String> conversationKeys,
    String? matchInboundFromPeerId,
  }) async {
    final params = <String, dynamic>{
      "app_namespace": appNamespace,
      "conversation_keys": conversationKeys,
      if (matchInboundFromPeerId != null && matchInboundFromPeerId.trim().isNotEmpty)
        "match_inbound_from_peer_id": matchInboundFromPeerId.trim(),
    };
    if (usesDaemon) {
      final r = await GhalBolDaemonClient.instance.callState(
        DaemonMethod.p2pTranscriptLoadMerged,
        params: params,
      );
      if (r["ok"] != true) return (revision: 0, lines: <Map<String, dynamic>>[]);
      final revRaw = r["revision"];
      final revision = revRaw is num ? revRaw.toInt() : 0;
      final lines = r["lines"];
      if (lines is! List) return (revision: revision, lines: <Map<String, dynamic>>[]);
      final parsed = lines
          .whereType<Map>()
          .map((e) => Map<String, dynamic>.from(e))
          .toList();
      return (revision: revision, lines: parsed);
    }
    return GhalBolFfi.transcriptLoadThreadView(appNamespace, params);
  }

  /// Native **video** control plane (H.264 over `/ghal-bol/call-video/1.0.0`).
  static Future<Map<String, dynamic>> callVideo(Map<String, dynamic> config) async {
    if (usesDaemon) {
      return GhalBolDaemonClient.instance.callState(
        DaemonMethod.p2pCallVideo,
        params: config,
      );
    }
    return GhalBolFfi.p2pCallVideo(config);
  }

  static Future<Map<String, dynamic>> callVideoStart({
    required String callId,
    required String recipientPublicKeyHex,
    bool cameraEnabled = false,
  }) {
    return callVideo({
      "action": "start",
      "call_id": callId,
      "recipient_public_key_hex": recipientPublicKeyHex,
      "camera_enabled": cameraEnabled,
    });
  }

  static Future<Map<String, dynamic>> callVideoStop({required String callId}) {
    return callVideo({"action": "stop", "call_id": callId});
  }

  static Future<Map<String, dynamic>> callVideoSetCameraEnabled({
    required String callId,
    required bool enabled,
  }) {
    return callVideo({
      "action": "set_camera_enabled",
      "call_id": callId,
      "enabled": enabled,
    });
  }

  /// Desktop capture path after video start: `nokhwa` (daemon) or `flutter` (UI inject).
  static Future<Map<String, dynamic>> callVideoCaptureBackend() {
    return callVideo({"action": "capture_backend", "call_id": "probe"});
  }

  /// Push one camera frame from the UI into the desktop native video engine.
  /// `format` `i420` (planar) or packed `rgba`/`bgra` (converted to I420 natively in
  /// Rust — no Dart per-pixel loop); `stride` is the packed source row length in bytes.
  static Future<Map<String, dynamic>> callVideoPushCameraFrame({
    required String callId,
    required int width,
    required int height,
    required String dataBase64,
    String format = "i420",
    int? stride,
  }) async {
    final config = {
      "call_id": callId,
      "width": width,
      "height": height,
      "data_base64": dataBase64,
      "format": format,
      "stride": stride ?? width * 4,
    };
    if (usesDaemon) {
      // State socket — must not queue behind chat/poll on the main socket (~30 fps).
      return GhalBolDaemonClient.instance.callState(
        DaemonMethod.p2pCallVideoPushCameraFrame,
        params: config,
        ensureDaemon: false,
      );
    }
    return GhalBolFfi.p2pCallVideoPushCameraFrame(config);
  }

  /// Shm path + dimensions for GPU texture registration (`track`: `remote` or `local`).
  static Future<Map<String, dynamic>> callVideoTexture({
    required String callId,
    String track = "remote",
  }) async {
    final config = {
      "call_id": callId,
      "track": track,
    };
    if (usesDaemon) {
      return GhalBolDaemonClient.instance.callState(
        DaemonMethod.p2pCallVideoTexture,
        params: config,
        ensureDaemon: false,
      );
    }
    return GhalBolFfi.p2pCallVideoTexture(config);
  }

  /// Pull the latest decoded frame for rendering (`track`: `remote` or `local`).
  /// `format` `rgba` (default) returns packed RGBA8888 converted natively in Rust so
  /// the Flutter UI isolate does no per-pixel work; `i420` returns the raw planar payload.
  /// [maxEdge] downscales the display pull in Rust (encode/send stays full-res).
  static Future<Map<String, dynamic>> callVideoFrame({
    required String callId,
    int sinceGeneration = 0,
    String track = "remote",
    String format = "rgba",
    int maxEdge = 360,
  }) async {
    final config = {
      "call_id": callId,
      "since_generation": sinceGeneration,
      "track": track,
      "format": format,
      if (maxEdge > 0) "max_edge": maxEdge,
    };
    if (usesDaemon) {
      // State socket — must not queue behind desktop camera push on the main socket.
      return GhalBolDaemonClient.instance.callState(
        DaemonMethod.p2pCallVideoFrame,
        params: config,
        ensureDaemon: false,
      );
    }
    return GhalBolFfi.p2pCallVideoFrame(config);
  }

  static Map<String, dynamic>? _normalizePollEvent(Object? ev) {
    if (ev == null) return null;
    if (ev is Map<String, dynamic>) return ev;
    if (ev is Map) return Map<String, dynamic>.from(ev);
    return null;
  }

  static DateTime? _lastPollFailureLogAt;
  static DateTime? _lastDaemonRecoverAt;
  static int _consecutivePollFailures = 0;

  static Future<Map<String, dynamic>?> pollEventMap() async {
    Map<String, dynamic>? map;
    if (usesDaemon) {
      // Poll on the state socket so events are not stuck behind send_text_dm / sync RPCs.
      var r = await GhalBolDaemonClient.instance.callState(
        DaemonMethod.p2pPoll,
        ensureDaemon: false,
      );
      if (r["ok"] != true) {
        final err = r["error"]?.toString() ?? "";
        if (err.contains("daemon not running") ||
            err.contains("disconnected")) {
          GhalBolDaemonClient.invalidateProbeCache();
          if (!await GhalBolDaemonClient.probeDaemon(force: true)) {
            _logPollFailure("daemon not reachable (probe failed)");
            await _tryRecoverDaemon();
            return null;
          }
          r = await GhalBolDaemonClient.instance.callState(
            DaemonMethod.p2pPoll,
            ensureDaemon: false,
          );
        }
        if (r["ok"] != true) {
          _logPollFailure(r["error"]?.toString() ?? "p2p_poll RPC failed");
          return null;
        }
      }
      _consecutivePollFailures = 0;
      map = _normalizePollEvent(r["event"]);
    } else {
      map = GhalBolFfi.p2pPollEventMap();
    }
    if (map != null) {
      logP2pEvent(map);
      pollEventDispatcher?.call(map);
    }
    return map;
  }

  static void _logPollFailure(String reason) {
    _consecutivePollFailures++;
    final now = DateTime.now();
    final last = _lastPollFailureLogAt;
    if (last != null && now.difference(last).inSeconds < 30) return;
    _lastPollFailureLogAt = now;
    P2pFlowLog.issue(
      "poll_stalled",
      detail: "×$_consecutivePollFailures $reason",
      check: "Daemon unlock + P2P step=p2p_start + app_namespace",
    );
  }

  /// Reconnect RPC when poll socket is down (does not stop `:p2p` unless ping still fails).
  static Future<void> _tryRecoverDaemon() async {
    if (!usesDaemon) return;
    final now = DateTime.now();
    final last = _lastDaemonRecoverAt;
    if (last != null && now.difference(last).inSeconds < 20) return;
    _lastDaemonRecoverAt = now;
    await GhalBolDaemonClient.reconnectDaemon();
  }

  static Future<bool> waitNodeReady({
    Duration timeout = const Duration(seconds: 8),
  }) async {
    final deadline = DateTime.now().add(timeout);
    while (DateTime.now().isBefore(deadline)) {
      var drained = false;
      while (true) {
        final ev = await pollEventMap();
        if (ev == null) break;
        drained = true;
        final kind = ev["kind"]?.toString();
        if (kind == DaemonPollEventKind.nodeReady) return true;
        if (kind == DaemonPollEventKind.nodeStopped) return false;
      }
      if (!drained && !await isRunning()) return false;
      await Future<void>.delayed(const Duration(milliseconds: 50));
    }
    return await isRunning();
  }

  static Future<Map<String, dynamic>> deliveryConnectionStatus() async {
    if (usesDaemon) {
      return GhalBolDaemonClient.instance.callState(
        DaemonMethod.deliveryConnectionStatus,
      );
    }
    return {"ok": false, "error": "delivery requires daemon"};
  }

  static Future<Map<String, dynamic>> deliveryQuotaStatus() async {
    if (usesDaemon) {
      return GhalBolDaemonClient.instance.callState(
        DaemonMethod.deliveryQuotaStatus,
      );
    }
    return {"ok": false, "error": "delivery requires daemon"};
  }

  static Future<Map<String, dynamic>> deliveryMailboxList({
    bool includeExpired = true,
  }) async {
    if (usesDaemon) {
      return GhalBolDaemonClient.instance.callState(
        DaemonMethod.deliveryMailboxList,
        params: {"include_expired": includeExpired},
      );
    }
    return {"ok": false, "error": "delivery requires daemon"};
  }

  static Future<Map<String, dynamic>> deliveryExtendTtl({
    required String messageId,
    required int extendSecs,
  }) async {
    if (usesDaemon) {
      return GhalBolDaemonClient.instance.callState(
        DaemonMethod.deliveryExtendTtl,
        params: {"message_id": messageId, "extend_secs": extendSecs},
      );
    }
    return {"ok": false, "error": "delivery requires daemon"};
  }

  static Future<Map<String, dynamic>> deliveryResendMessage(String messageId) async {
    if (usesDaemon) {
      return GhalBolDaemonClient.instance.callState(
        DaemonMethod.deliveryResendMessage,
        params: {"message_id": messageId},
      );
    }
    return {"ok": false, "error": "delivery requires daemon"};
  }
}
