import "dart:io" show Platform;

import "package:ghal_bol_ui/user_flow_log.dart";
import "package:ghal_bol_ui/ghal_bol_ffi.dart";
import "package:ghal_bol_ui/p2p_event_log.dart";
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
        "p2p_start",
        params: {"config": config},
      );
    }
    return GhalBolFfi.p2pStartJson(config);
  }

  static Future<void> stop() async {
    if (usesDaemon) {
      await GhalBolDaemonClient.instance.call("p2p_stop");
      return;
    }
    GhalBolFfi.p2pStop();
  }

  static Future<bool> isRunning() async {
    if (usesDaemon) {
      if (!await GhalBolDaemonClient.probeDaemon()) return false;
      final r = await GhalBolDaemonClient.instance.call(
        "p2p_is_running",
        ensureDaemon: false,
      );
      return r["ok"] == true && r["running"] == true;
    }
    return GhalBolFfi.p2pIsRunning();
  }

  /// Coord lookup addrs only — does not re-run full [startJson] / register storm.
  static Future<Map<String, dynamic>> dialBootstrapPeers(
    List<String> bootstrapPeers,
  ) async {
    if (usesDaemon) {
      return GhalBolDaemonClient.instance.callState(
        "p2p_dial_bootstrap",
        params: {"bootstrap_peers": bootstrapPeers},
        ensureDaemon: true,
      );
    }
    return {"ok": false, "error": "dial_bootstrap requires daemon"};
  }

  static Future<void> registerDmPeer(String publicKeyHex) async {
    if (usesDaemon) {
      await GhalBolDaemonClient.instance.call(
        "p2p_register_dm_peer",
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
        "p2p_send_text_dm",
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
        "p2p_requeue_outbound_dm",
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

  /// When false, native must not send `ack_read` (app background / UI destroyed).
  static Future<Map<String, dynamic>> setAppAckReadEnabled(bool enabled) async {
    if (usesDaemon) {
      return GhalBolDaemonClient.instance.callState(
        "p2p_set_app_ack_read_enabled",
        params: {"enabled": enabled},
        ensureDaemon: true,
      );
    }
    return GhalBolFfi.p2pSetAppAckReadEnabled(enabled);
  }

  static Future<Map<String, dynamic>> setForegroundPeer(String? publicKeyHex) async {
    if (usesDaemon) {
      final pk = publicKeyHex?.trim() ?? "";
      return GhalBolDaemonClient.instance.callState(
        "p2p_set_foreground_peer",
        params: pk.isEmpty ? {} : {"public_key_hex": pk},
        ensureDaemon: true,
      );
    }
    return GhalBolFfi.p2pSetForegroundPeer(publicKeyHex);
  }

  /// Voice/video call signaling (`invite`, `accept`, `sdp_offer`, `ice`, `video_on`, …).
  static Future<Map<String, dynamic>> callSignal(Map<String, dynamic> config) async {
    if (usesDaemon) {
      return GhalBolDaemonClient.instance.call(
        "p2p_call_signal",
        params: config,
      );
    }
    return GhalBolFfi.p2pCallSignal(config);
  }

  static Future<Map<String, dynamic>> sendAckDm({
    required String recipientPublicKeyHex,
    required String refId,
    required String ackKind,
  }) async {
    if (usesDaemon) {
      return GhalBolDaemonClient.instance.call(
        "p2p_send_ack_dm",
        params: {
          "recipient_public_key_hex": recipientPublicKeyHex,
          "ref_id": refId,
          "ack_kind": ackKind,
        },
      );
    }
    return GhalBolFfi.p2pSendAckDm(
      recipientPublicKeyHex: recipientPublicKeyHex,
      refId: refId,
      ackKind: ackKind,
    );
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
        "p2p_poll",
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
            "p2p_poll",
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

  /// Hint `:p2p` / daemon that the OS default network may have changed (resume, VPN, Wi‑Fi).
  static Future<void> notifyNetworkChange() async {
    if (!usesDaemon) return;
    if (!await isRunning()) return;
    try {
      await GhalBolDaemonClient.instance.callState(
        "p2p_notify_network_change",
        params: {},
        ensureDaemon: false,
      );
    } catch (_) {}
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
        if (kind == "node_ready") return true;
        if (kind == "node_stopped") return false;
      }
      if (!drained && !await isRunning()) return false;
      await Future<void>.delayed(const Duration(milliseconds: 50));
    }
    return await isRunning();
  }
}
