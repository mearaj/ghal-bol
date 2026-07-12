import "dart:async";
import "dart:math";

import "package:flutter/foundation.dart";
import "package:flutter/material.dart";
import "package:flutter/services.dart" show MethodChannel;
import "package:permission_handler/permission_handler.dart";

import "package:ghal_bol_ui/app_log.dart";
import "package:ghal_bol_ui/call/call_desktop_native_camera.dart";
import "package:ghal_bol_ui/call/call_video_texture_pool.dart";
import "package:ghal_bol_ui/call/call_flow_log.dart";
import "package:ghal_bol_ui/call/call_incoming_alert.dart";
import "package:ghal_bol_ui/call/call_ringtone.dart";
import "package:ghal_bol_ui/call/ghal_bol_call.dart";
import "package:ghal_bol_ui/call/call_screen.dart";
import "package:ghal_bol_ui/contact_store.dart";
import "package:ghal_bol_ui/ghal_bol_constants.dart";
import "package:ghal_bol_ui/ghal_bol_p2p.dart";
import "package:ghal_bol_ui/identity_display_name.dart";
import "package:ghal_bol_ui/p2p_event_bridge.dart";
import "package:ghal_bol_ui/p2p_link_error_ui.dart";
import "package:ghal_bol_ui/network_helper.dart";
import "package:ghal_bol_ui/public_key_hex.dart";
import "package:wakelock_plus/wakelock_plus.dart";

/// Android native voice (Oboe via cpal + NDK opus, in `:p2p`). Native is the only
/// voice path now (WebRTC removed). Audio is clean on a headset; speaker echo until
/// hardware/SW AEC lands — see `docs/GHAL_BOL_CALL_NATIVE_V2.md`.
const bool kAndroidNativeVoice = true;

/// Platforms with hardware earpiece/speaker routing the user can toggle.
bool get callSpeakerToggleSupported =>
    !kIsWeb &&
    (defaultTargetPlatform == TargetPlatform.android ||
        defaultTargetPlatform == TargetPlatform.iOS);

enum CallUiPhase {
  idle,
  outgoingRinging,
  incomingRinging,
  connecting,
  connected,
  ended,
}

/// One active call; voice-first with optional in-call video.
class CallController {
  CallController._();
  static final CallController instance = CallController._();

  static final GlobalKey<NavigatorState> navigatorKey = GlobalKey<NavigatorState>();

  static final List<VoidCallback> _callEndedListeners = [];

  /// Hub/chat reloads transcript after a call (native :p2p may have patched disk).
  static void addCallEndedListener(VoidCallback listener) {
    if (!_callEndedListeners.contains(listener)) {
      _callEndedListeners.add(listener);
    }
  }

  static void removeCallEndedListener(VoidCallback listener) {
    _callEndedListeners.remove(listener);
  }

  static void _notifyCallEnded() {
    for (final l in List<VoidCallback>.from(_callEndedListeners)) {
      l();
    }
  }

  CallUiPhase phase = CallUiPhase.idle;
  String? callId;
  String? peerPublicKeyHex;
  String? peerDisplayName;
  bool isOutgoing = false;
  bool localVideoOn = false;
  bool remoteVideoOn = false;
  /// Device-local PiP layout (main ↔ corner). Never sent to the peer.
  bool videoMainShowsLocal = false;
  bool micMuted = false;
  bool speakerOn = false;
  bool onHold = false;
  String? statusMessage;

  bool _acceptSent = false;
  /// Set when [syncActiveCallFromNative] rebuilds UI after `:p2p` outlived the shell.
  bool callRestoredFromNative = false;
  bool callScreenVisible = false;
  bool _callScreenPushInFlight = false;

  /// Voice-engine tag (see `docs/GHAL_BOL_CALL_NATIVE_V2.md`). Voice rides the Rust
  /// media engine over the libp2p substream — the only voice path (no WebRTC).
  static const String _nativeVoiceTag = "native_v2";
  /// Video-engine tag (see `docs/GHAL_BOL_VIDEO_NATIVE_V1.md`). Video rides H.264
  /// over `/ghal-bol/call-video/1.0.0` — the only video path (no WebRTC).
  static const String _nativeVideoTag = "native_v1";
  bool _nativeVoiceActive = false;
  bool _nativeVideoActive = false;
  bool _endingCall = false;
  Timer? _connectFallbackTimer;
  Timer? _connectPollTimer;
  Timer? _presentUiTimer;
  Timer? _peerDropGraceTimer;
  int _peerDropGraceEpoch = 0;
  /// Android notification already shown for this `call_id` (one shot per invite).
  String? _alertShownForCallId;

  /// True when native voice media is running for the active call.
  bool get nativeVoiceInCall => _nativeVoiceActive && inCallActive;

  /// True when the native video engine is running (send and/or receive).
  bool get nativeVideoInCall => _nativeVideoActive && inCallActive;

  /// Wire platform notification tap → show call UI.
  static void install() {
    unawaited(CallIncomingAlert.dismiss());
    CallIncomingAlert.installPlatformHandlers(
      onOpenedFromNotification: ({publicKeyHex, displayName}) => instance.onAppForeground(
        publicKeyHex: publicKeyHex,
        displayName: displayName,
      ),
      onWindowClosedByUser: () => instance.onWindowClosedByUser(),
    );
  }

  bool _windowCloseInFlight = false;

  /// Linux GTK **close (X)** — stop camera/media, notify peer, then hide window.
  void onWindowClosedByUser() {
    unawaited(_handleWindowClosedByUser());
  }

  Future<void> _handleWindowClosedByUser() async {
    if (_windowCloseInFlight) return;
    _windowCloseInFlight = true;
    try {
      CallFlowLog.step("window_closed_by_user", {"phase": phase.name});
      if (phase != CallUiPhase.idle && phase != CallUiPhase.ended) {
        final notifyRemote = phase == CallUiPhase.connected ||
            phase == CallUiPhase.connecting ||
            phase == CallUiPhase.outgoingRinging ||
            (phase == CallUiPhase.incomingRinging && _acceptSent);
        await _endLocal(notifyRemote: notifyRemote, awaitNativeStop: true);
      } else {
        await _stopNativeCallIfStillActive();
      }
      callScreenVisible = false;
      await CallDesktopNativeCamera.stop();
      await P2pEventBridge.instance.onLinuxWindowClosedByUser();
      if (!kIsWeb && defaultTargetPlatform == TargetPlatform.linux) {
        await CallIncomingAlert.hideWindow();
      }
    } finally {
      // Belt-and-suspenders: never leave camera/voice running after GTK close (X).
      await _stopNativeCallIfStillActive();
      _windowCloseInFlight = false;
    }
  }

  /// PiP tap — layout on this device only; no call signals or native RPC.
  void toggleVideoMainLocal({
    required bool remoteOn,
    required bool localOn,
  }) {
    if (!remoteOn && !localOn) return;
    if (remoteOn && localOn) {
      videoMainShowsLocal = !videoMainShowsLocal;
    } else if (localOn) {
      videoMainShowsLocal = true;
    } else {
      videoMainShowsLocal = false;
    }
    _notify();
  }

  /// Call UI left the screen while media may still be up — stop camera and hang up.
  Future<void> onCallScreenDismissedWhileLive() async {
    if (_endingCall) return;
    if (inCallActive) {
      CallFlowLog.step("call_screen_dismissed_during_live", {"phase": phase.name});
      await _endLocal(notifyRemote: true, awaitNativeStop: true);
      return;
    }
    if (phase == CallUiPhase.incomingRinging ||
        phase == CallUiPhase.outgoingRinging) {
      CallFlowLog.step("call_screen_dismissed_during_ring", {"phase": phase.name});
      final notifyRemote = phase == CallUiPhase.outgoingRinging ||
          (phase == CallUiPhase.incomingRinging && _acceptSent);
      await _endLocal(notifyRemote: notifyRemote, awaitNativeStop: true);
      return;
    }
    await _stopNativeCallIfStillActive();
  }

  /// Invites older than this are never shown (prevents stale poll replay after remote hangup).
  static const int _maxLiveInviteAgeMs = 45 * 1000;

  /// Window raised after close (notification tap / launcher) — not mere alt-tab focus.
  void onAppForeground({String? publicKeyHex, String? displayName}) {
    P2pEventBridge.instance.onLinuxWindowRestoredFromClose();
    unawaited(syncActiveCallFromNative(
      hintPublicKeyHex: publicKeyHex,
      hintDisplayName: displayName,
    ).then((restored) {
      if (!restored) {
        if (inCallActive && phase == CallUiPhase.connected) {
          _tryPushCallScreen();
        } else if (phase == CallUiPhase.incomingRinging ||
            phase == CallUiPhase.outgoingRinging) {
          _ensureCallScreenVisible();
        }
      }
      if (inCallActive && localVideoOn) {
        final id = callId;
        if (id != null) {
          unawaited(CallDesktopNativeCamera.refreshCaptureBackend().then((_) async {
            if (CallDesktopNativeCamera.usesFlutterCapture) {
              await CallDesktopNativeCamera.start(callId: id);
            }
          }));
        }
      }
    }));
    P2pEventBridge.instance.drainNow();
  }

  /// Reconcile in-memory UI state with `:p2p` when the native call outlived the UI process.
  Future<bool> syncActiveCallFromNative({
    String? hintPublicKeyHex,
    String? hintDisplayName,
  }) async {
    try {
      final r = await GhalBolP2p.callStatus();
      if (r["ok"] != true) return false;
      if (r["ringing"] == true && r["phase"]?.toString() == "incoming_ringing") {
        final id = r["call_id"]?.toString();
        final pk = (r["peer_public_key_hex"]?.toString() ?? hintPublicKeyHex ?? "")
            .trim()
            .toLowerCase();
        final ringAge = r["ring_age_ms"];
        final ringAgeMs = ringAge is int ? ringAge : int.tryParse("$ringAge");
        if (ringAgeMs != null && ringAgeMs > _maxLiveInviteAgeMs) {
          CallFlowLog.step("incoming_ring_stale_native", {
            "age_ms": ringAgeMs.toString(),
          });
          unawaited(CallIncomingAlert.dismiss());
          return false;
        }
        if (id != null &&
            id.isNotEmpty &&
            isValidPublicKeyHex(pk) &&
            phase == CallUiPhase.idle) {
          if (hintDisplayName != null && hintDisplayName.trim().isNotEmpty) {
            peerDisplayName = hintDisplayName.trim();
          }
          await _beginIncomingRing(pk, id);
          return true;
        }
      }
      if (r["active"] != true) return false;
      final id = r["call_id"]?.toString();
      final pk = r["peer_public_key_hex"]?.toString().trim().toLowerCase();
      final voice = r["voice_active"] == true;
      final video = r["video_active"] == true;
      if (id == null || id.isEmpty || pk == null || !isValidPublicKeyHex(pk)) return false;
      if (!voice && !video) return false;

      final cameraOn = r["camera_on"] == true;
      final remoteOn = r["remote_video_on"] == true || video;
      final sameCall = callId == id && inCallActive;

      if (!sameCall) {
        callRestoredFromNative = true;
        callId = id;
        CallFlowLog.bindCall(id);
        peerPublicKeyHex = pk;
        isOutgoing = false;
        _acceptSent = true;
        _setPhase(CallUiPhase.connected, "native_restore");
        await _resolvePeerDisplayName(pk);
        unawaited(_setWakelock(true));
      }

      _nativeVoiceActive = voice;
      _nativeVideoActive = video;
      localVideoOn = cameraOn;
      remoteVideoOn = remoteOn;
      if (!sameCall || statusMessage == null || statusMessage!.isEmpty) {
        statusMessage = cameraOn
            ? "Call in progress — your camera is on"
            : "Call in progress";
      }
      CallFlowLog.step("call_restored_from_native", {
        "call_id": id,
        "voice": voice.toString(),
        "video": video.toString(),
        "camera_on": cameraOn.toString(),
      });
      if (video && cameraOn) {
        await CallDesktopNativeCamera.refreshCaptureBackend();
      }
      _notify();
      _tryPushCallScreen();
      return true;
    } catch (e) {
      CallFlowLog.issue("call_restore_failed", detail: e.toString());
      return false;
    }
  }

  /// Platforms where the Rust-native voice engine is wired up.
  bool get _supportsNativeVoice {
    if (kIsWeb) return false;
    switch (defaultTargetPlatform) {
      case TargetPlatform.linux:
      case TargetPlatform.macOS:
      case TargetPlatform.windows:
        return true;
      case TargetPlatform.android:
        return kAndroidNativeVoice;
      default:
        return false;
    }
  }

  /// Desktop + Android ship the native H.264 video engine (OpenH264 + Camera2/nokhwa).
  bool get _supportsNativeVideo {
    if (kIsWeb) return false;
    switch (defaultTargetPlatform) {
      case TargetPlatform.linux:
      case TargetPlatform.macOS:
      case TargetPlatform.windows:
      case TargetPlatform.android:
        return true;
      default:
        return false;
    }
  }

  /// Native video is the only video path; available wherever the engine is wired.
  bool get _willUseNativeVideo => _supportsNativeVideo;

  bool get showRemoteVideo => remoteVideoOn;

  bool get showLocalPreview => localVideoOn;

  /// Media is sealed with identity keys (native engine) from your private key ×
  /// [peerPublicKeyHex]. Native voice/video is always E2E.
  bool get callMediaE2eeActive => _nativeVoiceActive || _nativeVideoActive;

  /// Short fingerprint of the contact public key used for call media E2EE.
  String? get callMediaE2eePeerShort {
    final peer = peerPublicKeyHex?.trim().toLowerCase();
    if (peer == null || peer.isEmpty) return null;
    return CallFlowLog.shortPk(peer);
  }

  /// One-line label for the in-call encryption chip.
  String? get callMediaE2eeLabel {
    if (!callMediaE2eeActive) return null;
    final peer = callMediaE2eePeerShort;
    if (peer == null) return "End-to-end encrypted";
    return "End-to-end encrypted · contact key $peer";
  }

  void handlePollEvent(Map<String, dynamic> ev) {
    final kind = ev["kind"]?.toString() ?? "";
    if (kind == "call_media") {
      _handleCallMediaPollEvent(ev);
      return;
    }
    if (kind == "call_signal") {
      _handleCallSignalEvent(ev);
      return;
    }
    if (kind == "call_signal_sent") {
      _handleCallSignalSentEvent(ev);
      return;
    }
    if (kind == "chat_ready") {
      final pk = peerPublicKeyHex;
      if (pk != null && publicKeysEqual(publicKeyHexFromEvent(ev), pk)) {
        if (inCallActive) {
          final wasReconnecting = statusMessage == "Reconnecting…";
          _cancelPeerDropGrace(clearStatus: true);
          if (wasReconnecting) {
            unawaited(_restartCallMediaAfterReconnect());
          }
        } else if (isOutgoing && phase == CallUiPhase.outgoingRinging) {
          statusMessage = "Ringing… (link ready)";
          _notify();
        }
      }
    }
    if (kind == "dial_failed" && phase != CallUiPhase.idle) {
      final err = ev["error"]?.toString() ?? "dial failed";
      if (!isTransientP2pLinkError(err)) {
        statusMessage = networkAwareUserP2pError(err) ?? "Call link failed";
        _notify();
      }
    }
    // Fallback — native `call_media` call_ended is authoritative (Phase E).
    if (kind == "peer_disconnected" && inCallActive) {
      final pk = streamContactKeyFromEvent(ev);
      final peer = peerPublicKeyHex;
      if (pk.isNotEmpty && peer != null && publicKeysEqual(pk, peer)) {
        _schedulePeerDropGrace();
      }
    }
  }

  void _cancelPeerDropGrace({bool clearStatus = false}) {
    _peerDropGraceEpoch++;
    _peerDropGraceTimer?.cancel();
    _peerDropGraceTimer = null;
    if (clearStatus && statusMessage == "Reconnecting…") {
      statusMessage = null;
      _notify();
    }
  }

  void _schedulePeerDropGrace() {
    final epoch = ++_peerDropGraceEpoch;
    _peerDropGraceTimer?.cancel();
    statusMessage = "Reconnecting…";
    _notify();
    _peerDropGraceTimer = Timer(const Duration(seconds: 4), () async {
      if (epoch != _peerDropGraceEpoch || !inCallActive) return;
      final pk = peerPublicKeyHex;
      if (pk != null && bridge.isStreamReady(pk)) {
        _cancelPeerDropGrace(clearStatus: true);
        return;
      }
      await _onPeerLinkLostDuringCall();
    });
  }

  /// Native voice/video lifecycle from `:p2p` — UI reflects state only.
  void _handleCallMediaPollEvent(Map<String, dynamic> ev) {
    final state = ev["state"]?.toString() ?? "";
    final pk = ev["peer_public_key_hex"]?.toString().trim().toLowerCase() ?? "";
    final id = ev["call_id"]?.toString() ?? "";
    final peer = peerPublicKeyHex?.trim().toLowerCase();
    if (pk.isEmpty || peer == null || !publicKeysEqual(pk, peer)) return;
    if (id.isNotEmpty && callId != null && id != callId) return;

    switch (state) {
      case "voice_started":
        _nativeVoiceActive = true;
        _notify();
      case "voice_stopped":
        _nativeVoiceActive = false;
        _notify();
      case "video_started":
        _nativeVideoActive = true;
        _notify();
      case "video_stopped":
        _nativeVideoActive = false;
        localVideoOn = false;
        unawaited(CallDesktopNativeCamera.stop());
        _notify();
      case "remote_video_on":
        if (remoteVideoOn) break;
        remoteVideoOn = true;
        _notify();
      case "remote_video_off":
        if (!remoteVideoOn) break;
        remoteVideoOn = false;
        _notify();
      case "call_ended":
        if (!inCallActive) return;
        if (ev["reason"]?.toString() == "peer_disconnected") {
          _schedulePeerDropGrace();
          return;
        }
        statusMessage = "Call ended";
        _notify();
        unawaited(_endLocal(notifyRemote: false));
      default:
        break;
    }
  }

  Future<void> _onPeerLinkLostDuringCall() async {
    if (!inCallActive) return;
    CallFlowLog.step("peer_link_lost_during_call");
    statusMessage = "Call ended — peer disconnected";
    _notify();
    await _endLocal(notifyRemote: false);
  }

  /// DM link blip during a call — reopen native voice/video on the fresh stream.
  Future<void> _restartCallMediaAfterReconnect() async {
    if (!inCallActive) return;
    final hadVideo = localVideoOn || remoteVideoOn;
    final camera = localVideoOn;
    _nativeVoiceActive = false;
    _nativeVideoActive = false;
    await _startNativeVoice();
    if (hadVideo) {
      await _ensureNativeVideoEngine(startCamera: camera);
    }
  }

  void _handleCallSignalEvent(Map<String, dynamic> ev) {
    final fromPk = publicKeyHexFromEvent(ev);
    if (!isValidPublicKeyHex(fromPk)) return;
    final remoteCallId = ev["call_id"]?.toString() ?? "";
    final signal = ev["signal"]?.toString() ?? "";
    final payload = ev["payload"];
    final Map<String, dynamic> pl = payload is Map
        ? Map<String, dynamic>.from(payload)
        : <String, dynamic>{};

    final createdAtMs = _eventCreatedAtMs(ev);
    unawaited(
      _onRemoteSignal(fromPk, remoteCallId, signal, pl, createdAtMs: createdAtMs),
    );
  }

  int? _eventCreatedAtMs(Map<String, dynamic> ev) {
    final raw = ev["created_at_ms"];
    if (raw is int) return raw;
    return int.tryParse(raw?.toString() ?? "");
  }

  bool _isLiveInvite(int? createdAtMs) {
    if (createdAtMs == null || createdAtMs <= 0) return false;
    final age = DateTime.now().millisecondsSinceEpoch - createdAtMs;
    return age >= 0 && age <= _maxLiveInviteAgeMs;
  }

  Future<void> startOutgoing({
    required BuildContext context,
    required String peerPublicKeyHex,
    required String displayName,
  }) async {
    if (!GhalBolP2p.isAvailable) {
      _snack(context, "Calls need P2P — unlock and wait for network.");
      return;
    }
    final net = NetworkHelper.instance.snapshot.value;
    if (net.hasLiveSnapshot && net.appearsOffline) {
      _snack(context, "No internet connection.");
      return;
    }
    if (phase != CallUiPhase.idle) {
      _snack(context, "Already in a call.");
      return;
    }
    if (!await _ensureMicPermission(context)) return;
    if (!context.mounted) return;

    final pk = peerPublicKeyHex.trim().toLowerCase();
    await GhalBolP2p.registerDmPeer(pk);
    if (!context.mounted) return;

    final id = _newCallId();
    callId = id;
    CallFlowLog.bindCall(id);
    this.peerPublicKeyHex = pk;
    peerDisplayName = displayName;
    isOutgoing = true;
    CallFlowLog.step("user_start_outgoing", {
      "peer": CallFlowLog.shortPk(pk),
      "name": displayName,
    });
    _setPhase(CallUiPhase.outgoingRinging, "start_outgoing");
    statusMessage = "Connecting to peer…";
    _notify();

    _showCallScreen(context);
    unawaited(_runOutgoingSetup(pk, id));
  }

  /// Runs after the call UI is shown — must not block on [Navigator.push].
  Future<void> _runOutgoingSetup(String pk, String id) async {
    if (phase != CallUiPhase.outgoingRinging) return;
    // If the DM stream dropped, nudge native reconnect (LAN/mDNS is usually <2s).
    if (!bridge.isStreamReady(pk)) {
      await GhalBolP2p.registerDmPeer(pk);
      for (var i = 0; i < 8; i++) {
        if (phase != CallUiPhase.outgoingRinging) return;
        if (bridge.isStreamReady(pk)) break;
        bridge.drainNow();
        await Future<void>.delayed(const Duration(milliseconds: 250));
      }
    }
    statusMessage = "Calling…";
    _notify();
    await _sendInvite(pk, id);
  }

  P2pEventBridge get bridge => P2pEventBridge.instance;

  Future<void> _sendInvite(String pk, String id) async {
    final r = await GhalBolCall.send(
      recipientPublicKeyHex: pk,
      callId: id,
      signal: "invite",
      payload: {
        "media": "audio",
        if (_supportsNativeVoice) "voice_engine": _nativeVoiceTag,
        if (_supportsNativeVideo) "video_engine": _nativeVideoTag,
      },
    );
    if (r["ok"] != true) {
      final err = r["error"]?.toString() ?? "Could not start call";
      CallFlowLog.issue("invite_send_failed", detail: err);
      statusMessage = err;
      _notify();
      return;
    }
    CallFlowLog.step("invite_queued", {"peer": CallFlowLog.shortPk(pk)});
  }

  void _handleCallSignalSentEvent(Map<String, dynamic> ev) {
    final signal = ev["signal"]?.toString() ?? "";
    final sentCallId = ev["call_id"]?.toString() ?? "";
    final pk = ev["recipient_public_key_hex"]?.toString() ?? "";
    if (signal != "invite") return;
    if (callId == null || sentCallId != callId) return;
    if (!publicKeysEqual(peerPublicKeyHex, pk)) return;
    if (phase != CallUiPhase.outgoingRinging) return;
    CallFlowLog.step("invite_on_wire", {"peer": CallFlowLog.shortPk(pk)});
    statusMessage = "Ringing…";
    unawaited(CallRingtone.startOutgoing());
    _notify();
  }

  Future<void> _onRemoteSignal(
    String fromPk,
    String remoteCallId,
    String signal,
    Map<String, dynamic> payload, {
    int? createdAtMs,
  }) async {
    CallFlowLog.step("signal_rx", {
      "signal": signal,
      "from": CallFlowLog.shortPk(fromPk),
    });

    if (signal == "invite") {
      if (!_isLiveInvite(createdAtMs)) {
        CallFlowLog.step("invite_stale_dropped", {
          "from": CallFlowLog.shortPk(fromPk),
          "call_id": remoteCallId,
          "age_ms": createdAtMs == null
              ? "?"
              : (DateTime.now().millisecondsSinceEpoch - createdAtMs).toString(),
        });
        return;
      }
      if (phase == CallUiPhase.outgoingRinging &&
          publicKeysEqual(peerPublicKeyHex, fromPk)) {
        final ours = callId;
        if (ours != null && remoteCallId.compareTo(ours) < 0) {
          await _abandonOutgoingForGlare(fromPk, remoteCallId);
          return;
        }
        if (ours != null && remoteCallId.compareTo(ours) >= 0) {
          CallFlowLog.step("glare_keep_outgoing", {
            "ours": ours,
            "theirs": remoteCallId,
          });
          return;
        }
      }
      if (phase != CallUiPhase.idle || inCallActive) {
        CallFlowLog.step("invite_rejected_busy", {
          "from": CallFlowLog.shortPk(fromPk),
          "phase": phase.name,
        });
        unawaited(CallRingtone.stop());
        unawaited(CallIncomingAlert.dismiss());
        await GhalBolCall.send(
          recipientPublicKeyHex: fromPk,
          callId: remoteCallId,
          signal: "reject",
        );
        return;
      }
      try {
        final st = await GhalBolP2p.callStatus();
        if (st["ok"] == true && st["active"] == true) {
          CallFlowLog.step("invite_rejected_native_active", {
            "from": CallFlowLog.shortPk(fromPk),
          });
          unawaited(CallRingtone.stop());
          unawaited(CallIncomingAlert.dismiss());
          await GhalBolCall.send(
            recipientPublicKeyHex: fromPk,
            callId: remoteCallId,
            signal: "reject",
          );
          return;
        }
      } catch (_) {}
      await _beginIncomingRing(fromPk, remoteCallId);
      return;
    }

    if (callId == null ||
        remoteCallId != callId ||
        !publicKeysEqual(peerPublicKeyHex, fromPk)) {
      if (signal == "hangup" || signal == "reject") {
        await _endNativeCallIfSignalMatches(fromPk, remoteCallId);
      }
      return;
    }

    switch (signal) {
      case "accept":
        if (isOutgoing &&
            (phase == CallUiPhase.outgoingRinging || phase == CallUiPhase.connecting)) {
          unawaited(CallRingtone.stop());
          statusMessage = "Answered";
          _notify();
          _enterConnecting();
          await _startNativeVoice();
        }
        break;
      case "reject":
      case "hangup":
        CallFlowLog.step(
          signal == "reject" ? "remote_declined" : "remote_hangup",
        );
        await _endLocal(notifyRemote: false);
        statusMessage = signal == "reject" ? "Declined" : "Call ended";
        _notify();
        break;
      case "video_on":
        if (remoteVideoOn) break;
        remoteVideoOn = true;
        CallFlowLog.step("remote_video_on");
        unawaited(_ensureNativeVideoEngine());
        _notify();
        break;
      case "video_off":
        if (!remoteVideoOn) break;
        remoteVideoOn = false;
        CallFlowLog.step("remote_video_off");
        _notify();
        break;
    }
  }

  bool get inCallActive =>
      phase == CallUiPhase.connecting || phase == CallUiPhase.connected;

  void _enterConnecting() {
    if (phase == CallUiPhase.idle || phase == CallUiPhase.ended) return;
    unawaited(CallRingtone.stop());
    unawaited(CallIncomingAlert.dismiss());
    _presentUiTimer?.cancel();
    _presentUiTimer = null;
    _setPhase(CallUiPhase.connecting, "media_start");
    statusMessage = "Connecting audio…";
    _connectFallbackTimer?.cancel();
    _connectPollTimer?.cancel();
    _connectPollTimer = Timer.periodic(const Duration(milliseconds: 500), (_) {
      if (phase != CallUiPhase.connecting) {
        _connectPollTimer?.cancel();
        _connectPollTimer = null;
        return;
      }
      P2pEventBridge.instance.drainNow();
    });
    _refreshConnectingStatus();
    _connectFallbackTimer = Timer(const Duration(seconds: 20), () {
      if (phase == CallUiPhase.connecting) {
        CallFlowLog.issue(
          "media_connect_timeout",
          check: "grep Call/P2P call_media substream; native call_media stats",
          detail: "no native media connected after 20s",
        );
        statusMessage = "No audio link — check App log for call media";
        _notify();
      }
    });
    _notify();
  }

  void _refreshConnectingStatus() {
    if (phase != CallUiPhase.connecting) return;
    statusMessage = "Connecting audio…";
  }

  void _markMediaConnected() {
    if (phase == CallUiPhase.idle ||
        phase == CallUiPhase.ended ||
        phase == CallUiPhase.incomingRinging ||
        phase == CallUiPhase.outgoingRinging) {
      return;
    }
    _connectFallbackTimer?.cancel();
    _connectFallbackTimer = null;
    _setPhase(CallUiPhase.connected, "media_ready");
    statusMessage = null;
    if (_willUseNativeVideo) {
      unawaited(_ensureNativeVideoEngine());
    }
    _notify();
  }

  void _setPhase(CallUiPhase next, String trigger) {
    if (phase == next) return;
    final from = phase.name;
    phase = next;
    CallFlowLog.step("ui_phase", {
      "from": from,
      "to": next.name,
      "trigger": trigger,
      "outgoing": isOutgoing.toString(),
    });
    _syncCallSessionExtras();
  }

  void _syncCallSessionExtras() {
    final keepAwake = phase == CallUiPhase.incomingRinging ||
        phase == CallUiPhase.outgoingRinging ||
        phase == CallUiPhase.connecting ||
        phase == CallUiPhase.connected;
    unawaited(_setWakelock(keepAwake));
  }

  Future<void> _setWakelock(bool on) async {
    try {
      if (on) {
        await WakelockPlus.enable();
      } else {
        await WakelockPlus.disable();
      }
    } catch (_) {}
  }

  Future<void> acceptIncoming() async {
    final pk = peerPublicKeyHex;
    final id = callId;
    if (pk == null || id == null || phase != CallUiPhase.incomingRinging) return;
    if (_acceptSent) return;
    _acceptSent = true;
    final ctx = navigatorKey.currentContext;
    if (ctx == null || !await _ensureMicPermission(ctx)) {
      _acceptSent = false;
      return;
    }
    _enterConnecting();

    final acceptPayload = <String, dynamic>{};
    if (_supportsNativeVoice) acceptPayload["voice_engine"] = _nativeVoiceTag;
    if (_supportsNativeVideo) acceptPayload["video_engine"] = _nativeVideoTag;
    final r = await GhalBolCall.send(
      recipientPublicKeyHex: pk,
      callId: id,
      signal: "accept",
      payload: acceptPayload,
    );
    if (r["ok"] != true) {
      final err = r["error"]?.toString() ?? "Accept failed";
      CallFlowLog.issue("accept_send_failed", detail: err);
      statusMessage = err;
      _acceptSent = false;
      _notify();
      return;
    }
    CallFlowLog.step("accept_sent", {"voice": "native"});
    await _startNativeVoice();
  }

  Future<void> rejectIncoming() async {
    final pk = peerPublicKeyHex;
    final id = callId;
    if (pk == null || id == null) return;
    CallFlowLog.step("user_reject");
    await GhalBolCall.send(recipientPublicKeyHex: pk, callId: id, signal: "reject");
    await _endLocal(notifyRemote: false);
  }

  Future<void> hangUp() async {
    if (phase == CallUiPhase.idle || phase == CallUiPhase.ended) {
      await _dismissCallScreens();
      return;
    }
    CallFlowLog.step("user_hangup");
    await _endLocal(notifyRemote: true);
  }

  Future<void> toggleMute() async {
    if (!inCallActive || !_nativeVoiceActive) return;
    micMuted = !micMuted;
    final id = callId;
    if (id != null) {
      await GhalBolP2p.callMediaSetMicMuted(callId: id, muted: micMuted);
    }
    _notify();
  }

  Future<void> toggleSpeaker() async {
    if (!inCallActive || !_nativeVoiceActive) return;
    if (!callSpeakerToggleSupported) return;
    speakerOn = !speakerOn;
    final id = callId;
    if (id != null) {
      final r = await GhalBolP2p.callMediaSetSpeaker(
        callId: id,
        speakerOn: speakerOn,
      );
      if (r["ok"] != true) {
        speakerOn = !speakerOn;
        CallFlowLog.issue(
          "native_speaker_toggle",
          detail: r["error"]?.toString() ?? "set_speaker failed",
        );
      }
    }
    _notify();
  }

  Future<void> toggleHold() async {
    if (!inCallActive || !_nativeVoiceActive) return;
    onHold = !onHold;
    micMuted = onHold;
    final id = callId;
    if (id != null) {
      await GhalBolP2p.callMediaSetMicMuted(callId: id, muted: onHold);
    }
    _notify();
  }

  Future<void> toggleVideo() async {
    if (!inCallActive || !_nativeVoiceActive) return;
    if (!_willUseNativeVideo) {
      statusMessage =
          "Video isn't available — both peers need a native video build.";
      _notify();
      return;
    }
    if (localVideoOn) {
      CallFlowLog.step("user_video_off");
      final id = callId;
      final pk = peerPublicKeyHex;
      if (id != null) {
        await GhalBolP2p.callVideoSetCameraEnabled(callId: id, enabled: false);
      }
      await CallDesktopNativeCamera.stop();
      if (pk != null && id != null) {
        await GhalBolCall.send(
          recipientPublicKeyHex: pk,
          callId: id,
          signal: "video_off",
        );
      }
      localVideoOn = false;
      statusMessage = null;
      _notify();
      return;
    }
    final ctx = navigatorKey.currentContext;
    if (ctx != null && !await _ensureCameraPermission(ctx)) return;
    CallFlowLog.step("user_video_on");
    await _ensureNativeVideoEngine(startCamera: true);
    if (!_nativeVideoActive) {
      statusMessage = "Video could not start — rebuild native lib and sync daemon";
      _notify();
      return;
    }
    await CallDesktopNativeCamera.refreshCaptureBackend();
    final id = callId;
    final pk = peerPublicKeyHex;
    if (CallDesktopNativeCamera.usesFlutterCapture && id != null) {
      try {
        await CallDesktopNativeCamera.start(callId: id);
      } catch (_) {
        statusMessage = "Camera unavailable — check webcam / PipeWire";
        await GhalBolP2p.callVideoSetCameraEnabled(callId: id, enabled: false);
        _notify();
        return;
      }
    }
    if (pk != null && id != null) {
      await GhalBolCall.send(
        recipientPublicKeyHex: pk,
        callId: id,
        signal: "video_on",
      );
    }
    localVideoOn = true;
    statusMessage = null;
    _notify();
  }

  /// Start the Rust-native voice engine for this call (no WebRTC). Both sides
  /// open their own media substream; audio rides the existing libp2p link.
  Future<void> _startNativeVoice() async {
    final pk = peerPublicKeyHex;
    final id = callId;
    if (pk == null || id == null) return;
    _nativeVoiceActive = true;
    CallFlowLog.step("native_voice_start", {"peer": CallFlowLog.shortPk(pk)});
    // Android: the engine records the mic from `:p2p`, which needs the microphone
    // FGS type. Re-promote the service now that RECORD_AUDIO is granted (P6).
    await _ensureAndroidMicForegroundService();
    final r = await GhalBolP2p.callMediaStart(
      callId: id,
      recipientPublicKeyHex: pk,
    );
    if (r["ok"] != true) {
      _nativeVoiceActive = false;
      final err = r["error"]?.toString() ?? "Could not start audio";
      CallFlowLog.issue(
        "native_voice_failed",
        check: "native lib rebuilt (ghal_bol_core_ffi_p2p_call_media); daemon synced",
        detail: err,
      );
      statusMessage = "Audio error — $err";
      _notify();
      return;
    }
    // No ICE handshake on the native path: the substream is already up, so the
    // call is live. Audio flowing is confirmed by the native `call_media` stats log.
    _markMediaConnected();
  }

  /// Start (or attach to) the native H.264 video engine. [startCamera] turns the
  /// local camera on at start; otherwise receive-only until the user toggles video.
  Future<void> _ensureNativeVideoEngine({bool startCamera = false}) async {
    if (!_willUseNativeVideo) return;
    final pk = peerPublicKeyHex;
    final id = callId;
    if (pk == null || id == null) return;
    if (_nativeVideoActive) {
      if (startCamera) {
        await GhalBolP2p.callVideoSetCameraEnabled(callId: id, enabled: true);
      }
      return;
    }
    CallFlowLog.step("native_video_start", {
      "peer": CallFlowLog.shortPk(pk),
      "camera": startCamera.toString(),
    });
    if (CallDesktopNativeCamera.usesFlutterCapture) {
      unawaited(CallDesktopNativeCamera.warmup());
    }
    await _ensureAndroidMicForegroundService();
    final r = await GhalBolP2p.callVideoStart(
      callId: id,
      recipientPublicKeyHex: pk,
      cameraEnabled: startCamera,
    );
    if (r["ok"] != true) {
      final err = r["error"]?.toString() ?? "Could not start video";
      CallFlowLog.issue(
        "native_video_failed",
        check: "native lib rebuilt (ghal_bol_core_ffi_p2p_call_video); daemon synced",
        detail: err,
      );
      return;
    }
    await CallDesktopNativeCamera.refreshCaptureBackend();
    if (startCamera && CallDesktopNativeCamera.usesFlutterCapture) {
      try {
        await CallDesktopNativeCamera.start(callId: id);
      } catch (_) {}
    }
    _nativeVideoActive = true;
    _notify();
  }

  /// Android only: nudge `GhalBolP2pService` to re-promote itself with the
  /// microphone foreground-service type so the `:p2p` process may record audio.
  /// No-op (and harmless) on other platforms. Idempotent — does not restart libp2p.
  Future<void> _ensureAndroidMicForegroundService() async {
    if (kIsWeb || defaultTargetPlatform != TargetPlatform.android) return;
    try {
      await const MethodChannel("ghal_bol/p2p_daemon")
          .invokeMethod<String>("startP2pService");
    } catch (e) {
      CallFlowLog.issue("android_mic_fgs", detail: e.toString());
    }
  }

  /// UI state was lost (app restart) but `:p2p` may still be in the call — stop native + camera.
  Future<void> _endNativeCallIfSignalMatches(
    String fromPk,
    String remoteCallId,
  ) async {
    try {
      final r = await GhalBolP2p.callStatus();
      if (r["ok"] != true || r["active"] != true) return;
      final activeId = r["call_id"]?.toString() ?? "";
      final activePk =
          r["peer_public_key_hex"]?.toString().trim().toLowerCase() ?? "";
      if (activeId.isEmpty || activeId != remoteCallId) return;
      if (isValidPublicKeyHex(activePk) && !publicKeysEqual(activePk, fromPk)) return;
      CallFlowLog.step("native_call_end_orphan_signal", {
        "call_id": activeId,
        "from": CallFlowLog.shortPk(fromPk),
      });
      callId = activeId;
      CallFlowLog.bindCall(activeId);
      peerPublicKeyHex = activePk.isNotEmpty ? activePk : fromPk;
      _nativeVoiceActive = r["voice_active"] == true;
      _nativeVideoActive = r["video_active"] == true;
      localVideoOn = r["camera_on"] == true;
      remoteVideoOn = r["remote_video_on"] == true || _nativeVideoActive;
      if (phase == CallUiPhase.idle || phase == CallUiPhase.ended) {
        _setPhase(CallUiPhase.connected, "native_orphan_hangup");
      }
      await _endLocal(notifyRemote: false, awaitNativeStop: true);
      statusMessage = "Call ended";
      _notify();
    } catch (e) {
      CallFlowLog.issue("native_call_end_orphan_failed", detail: e.toString());
    }
  }

  Future<void> _stopNativeCallIfStillActive() async {
    try {
      final r = await GhalBolP2p.callStatus();
      if (r["ok"] != true || r["active"] != true) return;
      final activeId = r["call_id"]?.toString();
      final activePk =
          r["peer_public_key_hex"]?.toString().trim().toLowerCase();
      if (activeId == null || activeId.isEmpty) return;
      CallFlowLog.step("native_call_stop_window_close", {"call_id": activeId});
      callId = activeId;
      CallFlowLog.bindCall(activeId);
      if (activePk != null && isValidPublicKeyHex(activePk)) {
        peerPublicKeyHex = activePk;
      }
      _nativeVoiceActive = r["voice_active"] == true;
      _nativeVideoActive = r["video_active"] == true;
      localVideoOn = r["camera_on"] == true;
      await GhalBolP2p.callMediaStop(callId: activeId);
      await GhalBolP2p.callVideoStop(callId: activeId);
      await CallDesktopNativeCamera.stop();
      CallDesktopNativeCamera.resetCaptureBackend();
      unawaited(CallVideoTexturePool.releaseCall(activeId));
      if (peerPublicKeyHex != null) {
        unawaited(
          GhalBolCall.send(
            recipientPublicKeyHex: peerPublicKeyHex!,
            callId: activeId,
            signal: "hangup",
          ),
        );
      }
    } catch (e) {
      CallFlowLog.issue("native_call_stop_window_close_failed", detail: e.toString());
    }
  }

  Future<void> _endLocal({
    required bool notifyRemote,
    bool awaitNativeStop = false,
  }) async {
    if (_endingCall) return;
    if (phase == CallUiPhase.idle || phase == CallUiPhase.ended) {
      await _dismissCallScreens();
      return;
    }
    _cancelPeerDropGrace();
    _endingCall = true;
    final pk = peerPublicKeyHex;
    final id = callId;
    final stopVoice = _nativeVoiceActive;
    final stopVideo = _nativeVideoActive;
    // Drop in-call media flags and end the UI immediately — never block on hangup RPC.
    _nativeVoiceActive = false;
    _nativeVideoActive = false;
    _connectFallbackTimer?.cancel();
    _connectFallbackTimer = null;
    _connectPollTimer?.cancel();
    _connectPollTimer = null;
    _presentUiTimer?.cancel();
    _presentUiTimer = null;
    unawaited(CallRingtone.stop());
    unawaited(CallIncomingAlert.dismiss());
    unawaited(_setWakelock(false));
    CallFlowLog.step("call_end", {"notify_remote": notifyRemote.toString()});
    _setPhase(CallUiPhase.ended, "end_local");
    _notify();
    if (localVideoOn && notifyRemote && pk != null && id != null) {
      unawaited(
        GhalBolCall.send(
          recipientPublicKeyHex: pk,
          callId: id,
          signal: "video_off",
        ),
      );
    }
    if (notifyRemote && pk != null && id != null) {
      final hangup = GhalBolCall.send(
        recipientPublicKeyHex: pk,
        callId: id,
        signal: "hangup",
      );
      if (awaitNativeStop) {
        await hangup;
      } else {
        unawaited(hangup);
      }
    }
    if (stopVoice && id != null) {
      final stop = GhalBolP2p.callMediaStop(callId: id);
      if (awaitNativeStop) {
        await stop;
      } else {
        unawaited(stop);
      }
    }
    if (stopVideo && id != null) {
      final stop = GhalBolP2p.callVideoStop(callId: id);
      if (awaitNativeStop) {
        await stop;
      } else {
        unawaited(stop);
      }
    }
    await CallDesktopNativeCamera.stop();
    CallDesktopNativeCamera.resetCaptureBackend();
    if (id != null) {
      unawaited(CallVideoTexturePool.releaseCall(id));
    }
    _acceptSent = false;
    await _dismissCallScreens();
    _notifyCallEnded();
    Future<void>.delayed(const Duration(milliseconds: 400), _reset);
  }

  Future<void> _dismissCallScreens() async {
    final nav = navigatorKey.currentState;
    if (nav == null) return;
    if (!callScreenPushLikelyOpen && !callScreenVisible) return;
    for (var i = 0; i < 4 && nav.canPop(); i++) {
      if (!callScreenVisible && i > 0) break;
      nav.pop();
      await Future<void>.delayed(const Duration(milliseconds: 20));
    }
    _callScreenPushInFlight = false;
  }

  bool get callScreenPushLikelyOpen =>
      callScreenVisible || _callScreenPushInFlight;

  void _reset() {
    _endingCall = false;
    callRestoredFromNative = false;
    _setPhase(CallUiPhase.idle, "reset");
    callId = null;
    CallFlowLog.bindCall(null);
    peerPublicKeyHex = null;
    peerDisplayName = null;
    isOutgoing = false;
    localVideoOn = false;
    remoteVideoOn = false;
    videoMainShowsLocal = false;
    micMuted = false;
    speakerOn = false;
    onHold = false;
    statusMessage = null;
    _alertShownForCallId = null;
    _nativeVoiceActive = false;
    _nativeVideoActive = false;
    _notify();
  }

  final _listeners = <VoidCallback>[];

  void addListener(VoidCallback l) => _listeners.add(l);
  void removeListener(VoidCallback l) => _listeners.remove(l);

  void _notify() {
    for (final l in List<VoidCallback>.from(_listeners)) {
      l();
    }
  }

  void _showCallScreen(BuildContext context) {
    if (callScreenPushLikelyOpen) return;
    final nav = CallController.navigatorKey.currentState ?? Navigator.of(context);
    _callScreenPushInFlight = true;
    unawaited(
      nav
          .push<void>(
            MaterialPageRoute<void>(
              fullscreenDialog: true,
              builder: (_) => const CallScreen(),
            ),
          )
          .whenComplete(() {
        _callScreenPushInFlight = false;
      })
          .then((_) async {
        if (phase == CallUiPhase.idle || phase == CallUiPhase.ended) return;
        if (phase == CallUiPhase.connected || phase == CallUiPhase.connecting) {
          await _endLocal(notifyRemote: true, awaitNativeStop: true);
          return;
        }
        if (!inCallActive &&
            phase != CallUiPhase.outgoingRinging &&
            phase != CallUiPhase.incomingRinging) {
          return;
        }
        final notifyRemote = phase == CallUiPhase.outgoingRinging ||
            (phase == CallUiPhase.incomingRinging && _acceptSent);
        await _endLocal(notifyRemote: notifyRemote);
      }),
    );
  }

  Future<void> _beginIncomingRing(String fromPk, String remoteCallId) async {
    callId = remoteCallId;
    CallFlowLog.bindCall(remoteCallId);
    peerPublicKeyHex = fromPk;
    isOutgoing = false;
    await _resolvePeerDisplayName(fromPk);
    CallFlowLog.step("incoming_ring", {"from": CallFlowLog.shortPk(fromPk)});
    _setPhase(CallUiPhase.incomingRinging, "invite");
    statusMessage = "Incoming call";
    unawaited(CallRingtone.startIncoming());
    _notify();
    await _presentIncomingCall();
    if (defaultTargetPlatform != TargetPlatform.linux ||
        await CallIncomingAlert.isWindowVisible()) {
      _ensureCallScreenVisible();
    }
  }

  Future<void> _abandonOutgoingForGlare(String fromPk, String remoteCallId) async {
    final oldId = callId;
    CallFlowLog.step("glare_become_callee", {
      "ours": oldId ?? "?",
      "theirs": remoteCallId,
    });
    unawaited(CallRingtone.stop());
    if (oldId != null) {
      await GhalBolCall.send(
        recipientPublicKeyHex: fromPk,
        callId: oldId,
        signal: "hangup",
      );
    }
    await _beginIncomingRing(fromPk, remoteCallId);
  }

  Future<void> _resolvePeerDisplayName(String pk) async {
    if (peerDisplayName != null && peerDisplayName!.trim().isNotEmpty) return;
    try {
      final c = await ContactStore.findByPublicKey(
        appNamespace: kGhalBolAppNamespace,
        publicKeyHex: pk,
      );
      peerDisplayName = ghalBolIdName(
        publicKeyHex: pk,
        customAlias: c?.displayAlias,
      );
    } catch (_) {}
    peerDisplayName ??= "Contact";
  }

  Future<void> _presentIncomingCall() async {
    if (phase != CallUiPhase.incomingRinging) return;
    final pk = peerPublicKeyHex;
    final id = callId;
    if (pk == null || id == null) return;
    if (_alertShownForCallId == id) return;

    await _resolvePeerDisplayName(pk);
    final name = peerDisplayName ?? "Contact";
    _alertShownForCallId = id;

    if (defaultTargetPlatform == TargetPlatform.iOS) {
      await CallIncomingAlert.show(displayName: name, publicKeyHex: pk);
      return;
    }
    // Android: full-screen notification is posted from `:p2p` when the invite arrives.

    if (defaultTargetPlatform == TargetPlatform.linux) {
      // Visible: in-app ring only. Hidden: `incoming_call_notify` in ghal_bol_core_daemon.
      return;
    }

    // Other desktops: raise window (no separate OS notification layer yet).
    await CallIncomingAlert.presentWindow();
  }

  void _ensureCallScreenVisible() {
    if (callScreenPushLikelyOpen) return;
    if (phase != CallUiPhase.incomingRinging &&
        phase != CallUiPhase.outgoingRinging) {
      return;
    }
    _presentUiTimer?.cancel();
    _tryPushCallScreen();
    _presentUiTimer = Timer.periodic(const Duration(milliseconds: 350), (_) {
      if (callScreenPushLikelyOpen ||
          (phase != CallUiPhase.incomingRinging &&
              phase != CallUiPhase.outgoingRinging)) {
        _presentUiTimer?.cancel();
        _presentUiTimer = null;
        return;
      }
      _tryPushCallScreen();
    });
  }

  void _tryPushCallScreen() {
    if (callScreenPushLikelyOpen) return;
    final nav = navigatorKey.currentState;
    if (nav == null) return;
    final showForActiveCall =
        inCallActive && phase == CallUiPhase.connected;
    if (phase != CallUiPhase.incomingRinging &&
        phase != CallUiPhase.outgoingRinging &&
        !showForActiveCall) {
      return;
    }
    _callScreenPushInFlight = true;
    nav
        .push<void>(
          MaterialPageRoute<void>(
            fullscreenDialog: true,
            builder: (_) => const CallScreen(),
          ),
        )
        .whenComplete(() {
      _callScreenPushInFlight = false;
    })
        .then((_) async {
      if (phase == CallUiPhase.idle || phase == CallUiPhase.ended) return;
      if (phase == CallUiPhase.connected || phase == CallUiPhase.connecting) {
        await _endLocal(notifyRemote: true, awaitNativeStop: true);
        return;
      }
      if (!inCallActive &&
          phase != CallUiPhase.outgoingRinging &&
          phase != CallUiPhase.incomingRinging) {
        return;
      }
      final notifyRemote = phase == CallUiPhase.outgoingRinging ||
          (phase == CallUiPhase.incomingRinging && _acceptSent);
      await _endLocal(notifyRemote: notifyRemote);
    });
  }

  static String _newCallId() {
    final r = Random();
    return "call-${DateTime.now().millisecondsSinceEpoch.toRadixString(16)}-${r.nextInt(0xFFFFFF).toRadixString(16)}";
  }

  bool get _usesMobilePermissions =>
      !kIsWeb &&
      (defaultTargetPlatform == TargetPlatform.android ||
          defaultTargetPlatform == TargetPlatform.iOS);

  Future<bool> _ensureMicPermission(BuildContext context) async {
    if (!_usesMobilePermissions) return true;
    try {
      final s = await Permission.microphone.request();
      if (s.isGranted) return true;
    } catch (e) {
      AppLog.instance.w("Call", "microphone permission plugin missing: $e");
      return true;
    }
    if (!context.mounted) return false;
    _snack(context, "Microphone permission is required for calls.");
    return false;
  }

  Future<bool> _ensureCameraPermission(BuildContext context) async {
    if (!_usesMobilePermissions) return true;
    try {
      final s = await Permission.camera.request();
      if (s.isGranted) return true;
    } catch (e) {
      AppLog.instance.w("Call", "camera permission plugin missing: $e");
      return true;
    }
    if (!context.mounted) return false;
    _snack(context, "Camera permission is required for video.");
    return false;
  }

  void _snack(BuildContext context, String msg) {
    if (!context.mounted) return;
    ScaffoldMessenger.of(context).showSnackBar(SnackBar(content: Text(msg)));
  }
}
