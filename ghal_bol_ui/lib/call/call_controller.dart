import "dart:async";
import "dart:math";

import "package:flutter/foundation.dart";
import "package:flutter/material.dart";
import "package:permission_handler/permission_handler.dart";

import "package:ghal_bol_ui/app_log.dart";
import "package:flutter_webrtc/flutter_webrtc.dart";
import "package:ghal_bol_ui/call/call_desktop_media.dart";
import "package:ghal_bol_ui/call/call_flow_log.dart";
import "package:ghal_bol_ui/call/call_incoming_alert.dart";
import "package:ghal_bol_ui/call/call_ringtone.dart";
import "package:ghal_bol_ui/call/call_webrtc.dart";
import "package:ghal_bol_ui/call/ghal_bol_call.dart";
import "package:ghal_bol_ui/call/call_screen.dart";
import "package:ghal_bol_ui/contact_store.dart";
import "package:ghal_bol_ui/ghal_bol_constants.dart";
import "package:ghal_bol_ui/ghal_bol_p2p.dart";
import "package:ghal_bol_ui/identity_display_name.dart";
import "package:ghal_bol_ui/p2p_event_bridge.dart";
import "package:ghal_bol_ui/p2p_link_error_ui.dart";
import "package:ghal_bol_ui/public_key_hex.dart";
import "package:wakelock_plus/wakelock_plus.dart";

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

  CallUiPhase phase = CallUiPhase.idle;
  String? callId;
  String? peerPublicKeyHex;
  String? peerDisplayName;
  bool isOutgoing = false;
  bool localVideoOn = false;
  bool remoteVideoOn = false;
  bool micMuted = false;
  bool speakerOn = false;
  bool onHold = false;
  String? statusMessage;

  CallWebRtc? _webrtc;
  bool _acceptSent = false;
  bool callScreenVisible = false;
  bool _inviteSent = false;
  Timer? _connectFallbackTimer;
  Timer? _connectPollTimer;
  Timer? _presentUiTimer;
  final List<MapEntry<String, Map<String, dynamic>>> _deferredWebRtcSignals =
      [];

  /// Wire Android notification tap → show call UI.
  static void install() {
    CallIncomingAlert.installOpenedHandler(() {
      instance.onAppForeground();
    });
  }

  /// App returned to foreground — show pending incoming call UI.
  void onAppForeground() {
    if (phase == CallUiPhase.incomingRinging ||
        phase == CallUiPhase.outgoingRinging) {
      unawaited(_presentIncomingCall());
      _ensureCallScreenVisible();
    }
    P2pEventBridge.instance.drainNow();
  }

  CallWebRtc? get webrtc => _webrtc;

  bool get showRemoteVideo =>
      remoteVideoOn || (_webrtc?.remoteVideoActive ?? false);

  bool get showLocalPreview => localVideoOn;

  void handlePollEvent(Map<String, dynamic> ev) {
    final kind = ev["kind"]?.toString() ?? "";
    if (kind == "call_signal") {
      _handleCallSignalEvent(ev);
      return;
    }
    if (kind == "chat_ready" && isOutgoing && phase == CallUiPhase.outgoingRinging) {
      final pk = peerPublicKeyHex;
      if (pk != null && publicKeysEqual(publicKeyHexFromEvent(ev), pk)) {
        statusMessage = "Ringing… (link ready)";
        _notify();
      }
    }
    if (kind == "dial_failed" && phase != CallUiPhase.idle) {
      final err = ev["error"]?.toString() ?? "dial failed";
      if (!isTransientP2pLinkError(err)) {
        statusMessage = shortUserP2pError(err) ?? "Call link failed";
        _notify();
      }
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

    unawaited(_onRemoteSignal(fromPk, remoteCallId, signal, pl));
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
    _inviteSent = false;
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
    CallFlowLog.step("wait_stream_ready", {"peer": CallFlowLog.shortPk(pk)});
    if (!await _waitForStreamReady(pk)) {
      if (phase == CallUiPhase.outgoingRinging) {
        CallFlowLog.issue(
          "peer_not_reachable",
          check: "chat must show connected; same network or coord",
          detail: "stream_ready timeout",
        );
        statusMessage =
            "Peer not reachable — wait until chat works, then try again.";
        _notify();
      }
      return;
    }
    CallFlowLog.step("stream_ready", {"peer": CallFlowLog.shortPk(pk)});
    if (phase != CallUiPhase.outgoingRinging) return;
    statusMessage = "Ringing…";
    _notify();
    await _sendInvite(pk, id);
  }

  Future<void> _sendInvite(String pk, String id) async {
    final r = await GhalBolCall.send(
      recipientPublicKeyHex: pk,
      callId: id,
      signal: "invite",
      payload: {"media": "audio"},
    );
    if (r["ok"] != true) {
      final err = r["error"]?.toString() ?? "Could not start call";
      CallFlowLog.issue("invite_send_failed", detail: err);
      statusMessage = err;
      _notify();
      return;
    }
    _inviteSent = true;
    CallFlowLog.step("invite_sent", {"peer": CallFlowLog.shortPk(pk)});
    unawaited(CallRingtone.startOutgoing());
  }

  Future<bool> _waitForStreamReady(String publicKeyHex, {Duration timeout = const Duration(seconds: 45)}) async {
    final bridge = P2pEventBridge.instance;
    if (bridge.isStreamReady(publicKeyHex)) return true;
    final deadline = DateTime.now().add(timeout);
    while (DateTime.now().isBefore(deadline)) {
      if (bridge.isStreamReady(publicKeyHex)) return true;
      await Future<void>.delayed(const Duration(milliseconds: 400));
      if (phase == CallUiPhase.idle || phase == CallUiPhase.ended) return false;
    }
    return bridge.isStreamReady(publicKeyHex);
  }

  Future<void> _onRemoteSignal(
    String fromPk,
    String remoteCallId,
    String signal,
    Map<String, dynamic> payload,
  ) async {
    CallFlowLog.step("signal_rx", {
      "signal": signal,
      "from": CallFlowLog.shortPk(fromPk),
    });

    if (signal == "invite") {
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
      if (phase != CallUiPhase.idle) {
        CallFlowLog.step("invite_rejected_busy", {
          "from": CallFlowLog.shortPk(fromPk),
          "phase": phase.name,
        });
        await GhalBolCall.send(
          recipientPublicKeyHex: fromPk,
          callId: remoteCallId,
          signal: "reject",
        );
        return;
      }
      await _beginIncomingRing(fromPk, remoteCallId);
      return;
    }

    if (callId == null ||
        remoteCallId != callId ||
        !publicKeysEqual(peerPublicKeyHex, fromPk)) {
      return;
    }

    switch (signal) {
      case "accept":
        if (isOutgoing &&
            (phase == CallUiPhase.outgoingRinging || phase == CallUiPhase.connecting)) {
          _enterConnecting();
          await _startWebRtcAsCaller();
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
        remoteVideoOn = true;
        CallFlowLog.step("remote_video_on");
        await _webrtc?.refreshRemotePlayback();
        if (localVideoOn && !(_webrtc?.localVideoEnabled ?? false)) {
          unawaited(_webrtc?.enableVideo());
        }
        _notify();
        break;
      case "video_off":
        remoteVideoOn = false;
        CallFlowLog.step("remote_video_off");
        _notify();
        break;
      case "sdp_offer":
      case "sdp_answer":
      case "ice":
        if (_shouldDeferWebRtcUntilAccept(signal)) {
          _deferredWebRtcSignals.add(
            MapEntry(signal, Map<String, dynamic>.from(payload)),
          );
          CallFlowLog.step("signal_deferred", {"signal": signal});
          break;
        }
        await _deliverWebRtcSignal(signal, payload);
        break;
    }
  }

  bool get inCallActive =>
      phase == CallUiPhase.connecting || phase == CallUiPhase.connected;

  bool _shouldDeferWebRtcUntilAccept(String signal) {
    return !isOutgoing &&
        phase == CallUiPhase.incomingRinging &&
        !_acceptSent &&
        (signal == "sdp_offer" || signal == "sdp_answer" || signal == "ice");
  }

  Future<void> _deliverWebRtcSignal(
    String signal,
    Map<String, dynamic> payload,
  ) async {
    if (_webrtc == null) {
      if (!isOutgoing) {
        await _initWebRtcCallee();
      } else {
        _webrtc ??= _newWebRtc();
        await _webrtc!.initRenderers();
      }
    }
    await _webrtc?.handleRemoteSignal(signal, payload);
  }

  Future<void> _flushDeferredWebRtcSignals() async {
    if (_deferredWebRtcSignals.isEmpty) return;
    final batch = List<MapEntry<String, Map<String, dynamic>>>.from(
      _deferredWebRtcSignals,
    );
    _deferredWebRtcSignals.clear();
    CallFlowLog.step("signal_flush", {"count": batch.length.toString()});
    for (final entry in batch) {
      await _deliverWebRtcSignal(entry.key, entry.value);
    }
  }

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
          check: "grep Call/P2P wire_rx sdp_answer; Call/WebRTC ice_*",
          detail: "no ICE connected after 20s",
        );
        statusMessage = "No audio link — check App log for sdp_answer / ice_*";
        _notify();
      }
    });
    _notify();
  }

  void _refreshConnectingStatus() {
    if (phase != CallUiPhase.connecting) return;
    final w = _webrtc;
    if (w == null || !w.remoteDescriptionSet) {
      statusMessage = "Waiting for caller…";
    } else if (!w.hasLocalAudio) {
      statusMessage = "Opening microphone…";
    } else {
      statusMessage = "Connecting audio…";
    }
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

    final r = await GhalBolCall.send(
      recipientPublicKeyHex: pk,
      callId: id,
      signal: "accept",
    );
    if (r["ok"] != true) {
      final err = r["error"]?.toString() ?? "Accept failed";
      CallFlowLog.issue("accept_send_failed", detail: err);
      statusMessage = err;
      _acceptSent = false;
      _notify();
      return;
    }
    CallFlowLog.step("accept_sent");
    await _initWebRtcCallee();
    _refreshConnectingStatus();
    await _flushDeferredWebRtcSignals();
    _refreshConnectingStatus();
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
    CallFlowLog.step("user_hangup");
    await _endLocal(notifyRemote: true);
  }

  Future<void> toggleMute() async {
    if (!inCallActive || _webrtc == null) return;
    micMuted = !micMuted;
    await _webrtc!.setMicMuted(micMuted);
    _notify();
  }

  Future<void> toggleSpeaker() async {
    if (!inCallActive || _webrtc == null) return;
    if (!callSpeakerToggleSupported) return;
    speakerOn = !speakerOn;
    await _webrtc!.setSpeakerOn(speakerOn);
    _notify();
  }

  Future<void> toggleHold() async {
    if (!inCallActive || _webrtc == null) return;
    onHold = !onHold;
    await _webrtc!.setMicMuted(onHold);
    await _webrtc!.setRemoteAudioPaused(onHold);
    micMuted = onHold;
    _notify();
  }

  Future<void> toggleVideo() async {
    if (!inCallActive || _webrtc == null) return;
    if (localVideoOn) {
      CallFlowLog.step("user_video_off");
      await _webrtc!.disableVideo();
      localVideoOn = false;
      _notify();
      return;
    }
    final ctx = navigatorKey.currentContext;
    if (ctx != null && !await _ensureCameraPermission(ctx)) return;
    CallFlowLog.step("user_video_on");
    try {
      await _webrtc!.enableVideo();
      localVideoOn = true;
      statusMessage = null;
    } catch (e, st) {
      _webrtc?.logError(e, st);
      CallFlowLog.issue(
        "camera_unavailable",
        check: "camera permission; desktop webcam / PipeWire",
        detail: e.toString(),
      );
      statusMessage = "Camera unavailable — check desktop camera / PipeWire";
      localVideoOn = false;
    }
    _notify();
  }

  Future<void> _startWebRtcAsCaller() async {
    if (!callWebRtcSupported) {
      statusMessage = "WebRTC not supported on this platform";
      _notify();
      return;
    }
    _webrtc ??= _newWebRtc();
    try {
      await _webrtc!.initRenderers();
      await _webrtc!.startAsCaller();
    } catch (e, st) {
      _webrtc?.logError(e, st);
      CallFlowLog.issue(
        "caller_media_failed",
        check: "mic permission; Call/Media audio_route lines",
      );
      statusMessage = "Media error — check microphone permission";
      _notify();
    }
  }

  Future<void> _initWebRtcCallee() async {
    if (!callWebRtcSupported) {
      statusMessage = "WebRTC not supported on this platform";
      _notify();
      return;
    }
    _webrtc ??= _newWebRtc();
    try {
      await _webrtc!.warmupIncomingCallee();
    } catch (e, st) {
      _webrtc?.logError(e, st);
      CallFlowLog.issue(
        "callee_media_failed",
        check: "mic permission; Call/Media audio_route lines",
      );
      statusMessage = "Media error — check microphone permission";
      _notify();
    }
  }

  CallWebRtc _newWebRtc() => CallWebRtc(
        politePeer: !isOutgoing,
        onSignal: _sendWebRtcSignal,
        onStreamsChanged: () {
          _refreshConnectingStatus();
          if (_webrtc?.remoteVideoActive ?? false) {
            remoteVideoOn = true;
          }
          _notify();
        },
        onIceConnectionState: _onIceState,
      );

  void _onIceState(RTCIceConnectionState state) {
    if (phase == CallUiPhase.idle || phase == CallUiPhase.ended) {
      return;
    }
    final name = state.toString().split(".").last;
    CallFlowLog.webrtc("ice_$name");
    switch (state) {
      case RTCIceConnectionState.RTCIceConnectionStateConnected:
      case RTCIceConnectionState.RTCIceConnectionStateCompleted:
        _markMediaConnected();
        break;
      case RTCIceConnectionState.RTCIceConnectionStateFailed:
        if (!inCallActive) return;
        CallFlowLog.issue(
          "ice_failed",
          check: "grep Call/WebRTC ice_; DM stream must be up",
        );
        statusMessage = "Audio connection failed";
        break;
      case RTCIceConnectionState.RTCIceConnectionStateDisconnected:
        if (!inCallActive) return;
        CallFlowLog.issue("ice_disconnected", check: "peer left or network drop");
        statusMessage = "Audio disconnected";
        break;
      default:
        break;
    }
    _notify();
  }

  Future<void> _sendWebRtcSignal(String signal, Map<String, dynamic> payload) async {
    final pk = peerPublicKeyHex;
    final id = callId;
    if (pk == null || id == null) return;
    final r = await GhalBolCall.send(
      recipientPublicKeyHex: pk,
      callId: id,
      signal: signal,
      payload: payload,
    );
    if (r["ok"] != true) {
      CallFlowLog.issue(
        "signal_tx_failed",
        check: "DM stream up; grep Call/P2P wire_rx on peer",
        detail: "$signal err=${r["error"]}",
      );
    }
  }

  Future<void> _endLocal({required bool notifyRemote}) async {
    final pk = peerPublicKeyHex;
    final id = callId;
    if (notifyRemote && pk != null && id != null) {
      await GhalBolCall.send(recipientPublicKeyHex: pk, callId: id, signal: "hangup");
    }
    _connectFallbackTimer?.cancel();
    _connectFallbackTimer = null;
    _connectPollTimer?.cancel();
    _connectPollTimer = null;
    _presentUiTimer?.cancel();
    _presentUiTimer = null;
    unawaited(CallRingtone.stop());
    unawaited(CallIncomingAlert.dismiss());
    unawaited(_setWakelock(false));
    await _webrtc?.dispose();
    _webrtc = null;
    _deferredWebRtcSignals.clear();
    CallDesktopMedia.clearCallSession();
    _acceptSent = false;
    _inviteSent = false;
    CallFlowLog.step("call_end", {"notify_remote": notifyRemote.toString()});
    _setPhase(CallUiPhase.ended, "end_local");
    _notify();
    final nav = navigatorKey.currentState;
    if (nav != null && nav.canPop()) {
      nav.pop();
    }
    Future<void>.delayed(const Duration(milliseconds: 400), _reset);
  }

  void _reset() {
    _setPhase(CallUiPhase.idle, "reset");
    callId = null;
    CallFlowLog.bindCall(null);
    peerPublicKeyHex = null;
    peerDisplayName = null;
    isOutgoing = false;
    localVideoOn = false;
    remoteVideoOn = false;
    micMuted = false;
    speakerOn = false;
    onHold = false;
    statusMessage = null;
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
    if (callScreenVisible) return;
    final nav = CallController.navigatorKey.currentState ?? Navigator.of(context);
    unawaited(
      nav
          .push<void>(
            MaterialPageRoute<void>(
              fullscreenDialog: true,
              builder: (_) => const CallScreen(),
            ),
          )
          .then((_) async {
        if (phase == CallUiPhase.idle || phase == CallUiPhase.ended) return;
        final notifyRemote = _inviteSent &&
            (phase == CallUiPhase.connected ||
                phase == CallUiPhase.connecting ||
                phase == CallUiPhase.incomingRinging);
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
    unawaited(_prewarmIncomingWebRtc());
    _notify();
    await _presentIncomingCall();
    _ensureCallScreenVisible();
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
    _inviteSent = false;
    await _webrtc?.dispose();
    _webrtc = null;
    await _beginIncomingRing(fromPk, remoteCallId);
  }

  Future<void> _prewarmIncomingWebRtc() async {
    if (!callWebRtcSupported) return;
    try {
      _webrtc ??= _newWebRtc();
      await _webrtc!.warmupIncomingCallee();
    } catch (e, st) {
      CallFlowLog.issue("callee_prewarm_failed", detail: e.toString());
      _webrtc?.logError(e, st);
    }
  }

  Future<void> _resolvePeerDisplayName(String pk) async {
    if (peerDisplayName != null && peerDisplayName!.trim().isNotEmpty) return;
    try {
      final c = await ContactStore.findByPublicKey(
        appNamespace: kGhalBolAndroidLibraryNamespace,
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
    final pk = peerPublicKeyHex;
    if (pk == null) return;
    await _resolvePeerDisplayName(pk);
    final name = peerDisplayName ?? "Contact";
    unawaited(CallIncomingAlert.presentWindow());
    unawaited(
      CallIncomingAlert.show(displayName: name, publicKeyHex: pk),
    );
  }

  void _ensureCallScreenVisible() {
    if (callScreenVisible) return;
    if (phase != CallUiPhase.incomingRinging &&
        phase != CallUiPhase.outgoingRinging) {
      return;
    }
    _presentUiTimer?.cancel();
    _tryPushCallScreen();
    _presentUiTimer = Timer.periodic(const Duration(milliseconds: 350), (_) {
      if (callScreenVisible ||
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
    if (callScreenVisible) return;
    final nav = navigatorKey.currentState;
    if (nav == null) return;
    nav.push<void>(
      MaterialPageRoute<void>(
        fullscreenDialog: true,
        builder: (_) => const CallScreen(),
      ),
    );
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
