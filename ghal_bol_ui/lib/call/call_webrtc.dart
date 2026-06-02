import "dart:async";

import "package:flutter/foundation.dart";
import "package:flutter_webrtc/flutter_webrtc.dart";
import "package:ghal_bol_ui/app_log.dart";
import "package:ghal_bol_ui/call/call_desktop_media.dart";
import "package:ghal_bol_ui/call/call_flow_log.dart";

/// WebRTC peer connection: audio-first, optional video via renegotiation.
class CallWebRtc {
  CallWebRtc({
    required this.onSignal,
    this.onStreamsChanged,
    this.onIceConnectionState,
    /// Callee rolls back a local offer when the remote peer sends one (glare).
    this.politePeer = false,
  });

  final Future<void> Function(String signal, Map<String, dynamic> payload) onSignal;
  final VoidCallback? onStreamsChanged;
  final void Function(RTCIceConnectionState state)? onIceConnectionState;
  final bool politePeer;

  RTCPeerConnection? _pc;
  MediaStream? _localStream;
  MediaStream? _remoteStream;
  bool _videoEnabled = false;
  bool _disposed = false;
  bool _remoteDescriptionSet = false;
  bool _pendingEnableVideo = false;
  final List<Map<String, dynamic>> _pendingIce = [];
  Future<void> _negotiationChain = Future<void>.value();

  final localRenderer = RTCVideoRenderer();
  final remoteRenderer = RTCVideoRenderer();

  MediaStream? get remoteStream => _remoteStream;
  bool get localVideoEnabled => _videoEnabled;
  bool get remoteVideoActive =>
      _remoteStream?.getVideoTracks().any((t) => t.enabled) ?? false;

  static const _iceServers = {
    "iceServers": [
      {"urls": "stun:stun.l.google.com:19302"},
    ],
  };

  /// Route call audio to speaker / communication mode (mobile). Call before getUserMedia.
  static Future<void> prepareCallAudio({bool speakerOn = true}) async {
    if (kIsWeb) return;
    try {
      if (CallDesktopMedia.isDesktopNative) {
        await CallDesktopMedia.prepareForCall();
      }
      if (WebRTC.platformIsAndroid) {
        await Helper.setAndroidAudioConfiguration(
          AndroidAudioConfiguration.communication,
        );
      }
      if (WebRTC.platformIsIOS) {
        await Helper.ensureAudioSession();
      }
      if (WebRTC.platformIsAndroid || WebRTC.platformIsIOS) {
        await Helper.setSpeakerphoneOn(speakerOn);
      }
    } catch (e) {
      CallFlowLog.issue("prepare_audio_failed", detail: e.toString());
    }
  }

  Future<void> initRenderers() async {
    CallFlowLog.webrtc("renderers_init");
    await localRenderer.initialize();
    await remoteRenderer.initialize();
  }

  Future<void> dispose() async {
    if (_disposed) return;
    CallFlowLog.webrtc("dispose");
    _disposed = true;
    _pendingIce.clear();
    try {
      await _localStream?.dispose();
    } catch (_) {}
    _localStream = null;
    try {
      await _pc?.close();
    } catch (_) {}
    _pc = null;
    await localRenderer.dispose();
    await remoteRenderer.dispose();
  }

  Future<void> _ensurePc() async {
    if (_pc != null) return;
    _pc = await createPeerConnection(_iceServers);
    _pc!.onIceCandidate = (c) {
      if (c.candidate == null || c.candidate!.isEmpty) return;
      unawaited(onSignal("ice", {
        "candidate": c.candidate,
        "sdpMid": c.sdpMid,
        "sdpMLineIndex": c.sdpMLineIndex,
      }));
    };
    _pc!.onIceConnectionState = (state) {
      onIceConnectionState?.call(state);
    };
    _pc!.onTrack = (ev) {
      unawaited(_attachRemoteTrack(ev));
    };
  }

  void _notifyStreams() => onStreamsChanged?.call();

  Future<void> _attachRemoteTrack(RTCTrackEvent ev) async {
    final track = ev.track;
    final kind = track.kind ?? "?";
    await _mergeRemoteTrack(track, ev.streams);
    CallFlowLog.webrtc("remote_track", {
      "kind": kind,
      "streams": ev.streams.length.toString(),
      "remote_audio": _remoteStream?.getAudioTracks().length.toString() ?? "0",
      "remote_video": _remoteStream?.getVideoTracks().length.toString() ?? "0",
    });
    await _bindRemotePlayback(forceRendererRefresh: kind == "video");
    _notifyStreams();
  }

  /// One combined remote [MediaStream] so audio + video renegotiation never
  /// replaces the renderer with an audio-only stream (common unified-plan bug).
  Future<void> _mergeRemoteTrack(
    MediaStreamTrack track,
    List<MediaStream> eventStreams,
  ) async {
    if (_remoteStream == null) {
      if (eventStreams.isNotEmpty) {
        _remoteStream = eventStreams.first;
      } else {
        _remoteStream = await createLocalMediaStream("remote");
      }
    } else if (eventStreams.isNotEmpty) {
      final incoming = eventStreams.first;
      if (incoming.id != _remoteStream!.id) {
        for (final t in incoming.getTracks()) {
          if (!_remoteStream!.getTracks().any((x) => x.id == t.id)) {
            await _remoteStream!.addTrack(t);
          }
        }
      }
    }
    if (!_remoteStream!.getTracks().any((t) => t.id == track.id)) {
      await _remoteStream!.addTrack(track);
    }
    track.enabled = true;
  }

  Future<void> _bindRemotePlayback({bool forceRendererRefresh = false}) async {
    final stream = _remoteStream;
    if (stream == null) return;
    for (final t in stream.getAudioTracks()) {
      t.enabled = true;
    }
    for (final t in stream.getVideoTracks()) {
      t.enabled = true;
    }
    final hasVideo = stream.getVideoTracks().isNotEmpty;
    // Re-bind after video renegotiation — GTK/Android often keep a stale texture
    // if [srcObject] was set on an audio-only stream first.
    if (forceRendererRefresh || hasVideo) {
      remoteRenderer.srcObject = null;
      await Future<void>.delayed(Duration.zero);
    }
    remoteRenderer.srcObject = stream;
    await CallDesktopMedia.bindRemoteAudioOutput(remoteRenderer);
  }

  /// UI may show remote video before/without a [video_on] signal — refresh bind.
  Future<void> refreshRemotePlayback() async {
    if (_disposed || _remoteStream == null) return;
    await _bindRemotePlayback(forceRendererRefresh: true);
    _notifyStreams();
  }

  Map<String, dynamic> _videoConstraints() {
    if (kIsWeb) return {"video": true};
    if (CallDesktopMedia.isDesktopNative) {
      return CallDesktopMedia.videoConstraints();
    }
    return {
      "video": {
        "width": {"ideal": 1280},
        "height": {"ideal": 720},
        "frameRate": {"ideal": 24},
      },
    };
  }

  Future<void> _ensureLocal({required bool video}) async {
    await _ensurePc();
    final constraints = <String, dynamic>{
      "audio": CallDesktopMedia.isDesktopNative
          ? await CallDesktopMedia.audioConstraints()
          : {
              "echoCancellation": true,
              "noiseSuppression": true,
              "autoGainControl": true,
            },
      if (video) ..._videoConstraints() else "video": false,
    };
    if (_localStream != null) {
      for (final t in _localStream!.getTracks()) {
        await t.stop();
      }
      await _localStream!.dispose();
      _localStream = null;
    }
    CallFlowLog.webrtc("get_user_media", {"video": video.toString()});
    try {
      _localStream = await navigator.mediaDevices.getUserMedia(constraints);
      final a = _localStream!.getAudioTracks().length;
      final v = _localStream!.getVideoTracks().length;
      CallFlowLog.webrtc("get_user_media_ok", {
        "audio_tracks": a.toString(),
        "video_tracks": v.toString(),
      });
      if (CallDesktopMedia.isDesktopNative && a > 0) {
        CallDesktopMedia.logCaptureTrack(_localStream!.getAudioTracks().first);
      }
    } catch (e, st) {
      CallFlowLog.issue(
        "get_user_media_failed",
        check: "mic permission; Call/Media enumerate + speaker_route",
        detail: "video=$video err=$e",
      );
      AppLog.instance.e("Call/WebRTC", "getUserMedia video=$video failed", e, st);
      rethrow;
    }
    _videoEnabled = video;
    if (video) {
      localRenderer.srcObject = _localStream;
    } else {
      localRenderer.srcObject = null;
    }
    for (final sender in await _pc!.getSenders()) {
      await _pc!.removeTrack(sender);
    }
    for (final track in _localStream!.getTracks()) {
      await _pc!.addTrack(track, _localStream!);
    }
    _notifyStreams();
  }

  Future<void> startAsCaller() async {
    CallFlowLog.webrtc("caller_start");
    await prepareCallAudio();
    await _ensureLocal(video: false);
    final offer = await _pc!.createOffer(_sdpMediaOptions);
    final local = await _setLocalDescriptionAndGather(offer);
    CallFlowLog.webrtc("sdp_offer_tx");
    await onSignal("sdp_offer", {"sdp": local.sdp, "type": local.type});
  }

  Future<void> handleRemoteSignal(String signal, Map<String, dynamic> payload) async {
    if (_disposed) return;
    await _enqueueNegotiation(() async {
      await _ensurePc();
      CallFlowLog.webrtc("signal_rx", {"signal": signal});
      switch (signal) {
        case "sdp_offer":
          await _handleRemoteOffer(payload);
          break;
        case "sdp_answer":
          await _handleRemoteAnswer(payload);
          break;
        case "ice":
          if (!_remoteDescriptionSet) {
            _pendingIce.add(Map<String, dynamic>.from(payload));
            CallFlowLog.webrtcDetail(
              "ice_buffered",
              "pending=${_pendingIce.length}",
            );
            return;
          }
          await _addIceCandidate(payload);
          break;
        default:
          break;
      }
    });
  }

  Future<void> _enqueueNegotiation(Future<void> Function() work) {
    final run = _negotiationChain.then((_) => work());
    _negotiationChain = run.catchError((_) {});
    return run;
  }

  Future<bool> _signalingStable() async {
    final state = await _pc?.getSignalingState();
    return state == RTCSignalingState.RTCSignalingStateStable;
  }

  Future<void> _waitSignalingStable({
    Duration timeout = const Duration(seconds: 12),
  }) async {
    final deadline = DateTime.now().add(timeout);
    while (DateTime.now().isBefore(deadline)) {
      if (_disposed) return;
      if (await _signalingStable()) return;
      await Future<void>.delayed(const Duration(milliseconds: 80));
    }
    CallFlowLog.issue(
      "signaling_not_stable",
      detail: "state=${await _pc?.getSignalingState()}",
    );
  }

  Future<bool> _rollbackLocalOfferForGlare() async {
    final state = await _pc?.getSignalingState();
    if (state != RTCSignalingState.RTCSignalingStateHaveLocalOffer) {
      return false;
    }
    if (!politePeer) {
      CallFlowLog.webrtc("glare_keep_local_offer");
      return false;
    }
    CallFlowLog.webrtc("glare_rollback");
    await _pc!.setLocalDescription(RTCSessionDescription("", "rollback"));
    return true;
  }

  static const _sdpMediaOptions = {
    "offerToReceiveAudio": true,
    "offerToReceiveVideo": true,
  };

  Future<void> _handleRemoteOffer(Map<String, dynamic> payload) async {
    final sdp = payload["sdp"]?.toString() ?? "";
    final type = payload["type"]?.toString() ?? "offer";
    if (sdp.isEmpty) return;
    final offer = RTCSessionDescription(sdp, type);
    final rolled = await _rollbackLocalOfferForGlare();
    if (!rolled) {
      final state = await _pc!.getSignalingState();
      if (state == RTCSignalingState.RTCSignalingStateHaveLocalOffer) {
        return;
      }
    }
    if (_pendingEnableVideo && !_videoEnabled) {
      await _addVideoTrack();
    }
    try {
      await _pc!.setRemoteDescription(offer);
    } catch (e, st) {
      CallFlowLog.issue("set_remote_offer_failed", detail: e.toString());
      AppLog.instance.e("Call/WebRTC", "setRemoteDescription(offer)", e, st);
      rethrow;
    }
    _remoteDescriptionSet = true;
    await _flushPendingIce();
    if (_localStream == null) {
      await prepareCallAudio();
      await _ensureLocal(video: false);
    }
    final answer = await _pc!.createAnswer(_sdpMediaOptions);
    final local = await _setLocalDescriptionAndGather(answer);
    CallFlowLog.webrtc("sdp_answer_tx");
    await onSignal("sdp_answer", {"sdp": local.sdp, "type": local.type});
    await _bindRemotePlayback(forceRendererRefresh: true);
    _notifyStreams();
    await _runPendingEnableVideo();
  }

  Future<void> _handleRemoteAnswer(Map<String, dynamic> payload) async {
    final sdp = payload["sdp"]?.toString() ?? "";
    final type = payload["type"]?.toString() ?? "answer";
    if (sdp.isEmpty) return;
    try {
      await _pc!.setRemoteDescription(RTCSessionDescription(sdp, type));
    } catch (e, st) {
      final msg = e.toString();
      if (msg.contains("m-lines") || msg.contains("wrong state")) {
        CallFlowLog.webrtc("stale_answer_ignored", {"detail": msg});
        return;
      }
      CallFlowLog.issue("set_remote_answer_failed", detail: msg);
      AppLog.instance.e("Call/WebRTC", "setRemoteDescription(answer)", e, st);
      rethrow;
    }
    _remoteDescriptionSet = true;
    await _flushPendingIce();
    await _bindRemotePlayback(forceRendererRefresh: true);
    _notifyStreams();
    await _runPendingEnableVideo();
  }

  Future<void> _flushPendingIce() async {
    if (_pc == null) return;
    final batch = List<Map<String, dynamic>>.from(_pendingIce);
    _pendingIce.clear();
    for (final p in batch) {
      await _addIceCandidate(p);
    }
  }

  Future<void> _addIceCandidate(Map<String, dynamic> payload) async {
    final cand = payload["candidate"]?.toString();
    if (cand == null || cand.isEmpty) return;
    try {
      await _pc!.addCandidate(
        RTCIceCandidate(
          cand,
          payload["sdpMid"]?.toString(),
          _iceMLineIndex(payload["sdpMLineIndex"]),
        ),
      );
    } catch (e, st) {
      CallFlowLog.issue("add_ice_failed", detail: e.toString());
      AppLog.instance.e("Call/WebRTC", "addIceCandidate failed", e, st);
    }
  }

  static int? _iceMLineIndex(dynamic v) {
    if (v == null) return null;
    if (v is int) return v;
    if (v is num) return v.toInt();
    return int.tryParse(v.toString());
  }

  /// Embed host/LAN candidates in SDP so calls work when trickle ICE is delayed in poll.
  Future<RTCSessionDescription> _setLocalDescriptionAndGather(
    RTCSessionDescription desc,
  ) async {
    final pc = _pc!;
    final done = Completer<void>();
    pc.onIceGatheringState = (state) {
      if (state == RTCIceGatheringState.RTCIceGatheringStateComplete &&
          !done.isCompleted) {
        done.complete();
      }
    };
    await pc.setLocalDescription(desc);
    try {
      await done.future.timeout(const Duration(seconds: 4));
      CallFlowLog.webrtcDetail("ice_gather", "complete");
    } catch (_) {
      CallFlowLog.webrtcDetail("ice_gather", "timeout — sending partial SDP");
    }
    final updated = await pc.getLocalDescription();
    return updated ?? desc;
  }

  /// Add camera without tearing down live audio senders (required for in-call video).
  Future<void> _addVideoTrack() async {
    if (_videoEnabled && (_localStream?.getVideoTracks().isNotEmpty ?? false)) {
      return;
    }
    CallFlowLog.webrtc("add_video_track");
    final videoOnly = await navigator.mediaDevices.getUserMedia({
      "audio": false,
      if (CallDesktopMedia.isDesktopNative)
        ...CallDesktopMedia.videoConstraints()
      else
        ..._videoConstraints(),
    });
    final videoTrack = videoOnly.getVideoTracks().first;
    if (CallDesktopMedia.isDesktopNative) {
      await CallDesktopMedia.prepareForCall();
    }
    _localStream ??= await createLocalMediaStream("local");
    await _localStream!.addTrack(videoTrack);
    await _pc!.addTrack(videoTrack, _localStream!);
    _videoEnabled = true;
    localRenderer.srcObject = _localStream;
    _notifyStreams();
  }

  Future<void> _removeVideoTrackDesktop() async {
    final stream = _localStream;
    final pc = _pc;
    if (stream == null || pc == null) return;
    for (final t in List<MediaStreamTrack>.from(stream.getVideoTracks())) {
      for (final sender in await pc.getSenders()) {
        if (sender.track?.id == t.id) {
          await pc.removeTrack(sender);
        }
      }
      await t.stop();
      try {
        await stream.removeTrack(t);
      } catch (_) {}
    }
  }

  Future<void> enableVideo() async {
    if (_videoEnabled) return;
    if (_disposed) return;
    await _ensurePc();
    if (!await _signalingStable()) {
      _pendingEnableVideo = true;
      CallFlowLog.webrtc("enable_video_deferred", {
        "state": (await _pc!.getSignalingState()).toString(),
      });
      return;
    }
    await _enqueueNegotiation(_runEnableVideo);
  }

  Future<void> _runPendingEnableVideo() async {
    if (!_pendingEnableVideo || _videoEnabled || _disposed) return;
    if (!await _signalingStable()) return;
    _pendingEnableVideo = false;
    await _enqueueNegotiation(_runEnableVideo);
  }

  Future<void> _runEnableVideo() async {
    if (_videoEnabled || _disposed) return;
    CallFlowLog.webrtc("enable_video");
    await _waitSignalingStable();
    if (_disposed || !await _signalingStable()) return;
    try {
      await _addVideoTrack();
    } catch (e, st) {
      CallFlowLog.issue(
        "add_video_track_failed",
        check: "camera permission; in-call video",
        detail: e.toString(),
      );
      AppLog.instance.e("Call/WebRTC", "addVideoTrack failed", e, st);
      rethrow;
    }
    await onSignal("video_on", {});
    final offer = await _pc!.createOffer(_sdpMediaOptions);
    final local = await _setLocalDescriptionAndGather(offer);
    await onSignal("sdp_offer", {"sdp": local.sdp, "type": local.type});
    CallFlowLog.webrtc("sdp_offer_tx");
    unawaited(
      Future<void>.delayed(const Duration(milliseconds: 600), () async {
        if (!_disposed) await refreshRemotePlayback();
      }),
    );
  }

  Future<void> disableVideo() async {
    if (!_videoEnabled) return;
    CallFlowLog.webrtc("disable_video");
    await onSignal("video_off", {});
    if (_pc != null) {
      await _removeVideoTrackDesktop();
    }
    _videoEnabled = false;
    _pendingEnableVideo = false;
    localRenderer.srcObject = null;
    _notifyStreams();
    if (_pc != null && await _signalingStable()) {
      await _enqueueNegotiation(() async {
        final offer = await _pc!.createOffer(_sdpMediaOptions);
        final local = await _setLocalDescriptionAndGather(offer);
        await onSignal("sdp_offer", {"sdp": local.sdp, "type": local.type});
      });
    }
  }

  Future<void> setMicMuted(bool muted) async {
    for (final t in _localStream?.getAudioTracks() ?? <MediaStreamTrack>[]) {
      t.enabled = !muted;
      if (!kIsWeb &&
          (WebRTC.platformIsAndroid || WebRTC.platformIsIOS)) {
        try {
          await Helper.setMicrophoneMute(muted, t);
        } catch (_) {}
      }
    }
  }

  Future<void> setSpeakerOn(bool on) async {
    if (kIsWeb) return;
    if (WebRTC.platformIsAndroid || WebRTC.platformIsIOS) {
      await Helper.setSpeakerphoneOn(on);
    }
  }

  Future<void> setRemoteAudioPaused(bool paused) async {
    for (final t in _remoteStream?.getAudioTracks() ?? <MediaStreamTrack>[]) {
      t.enabled = !paused;
    }
  }

  void logError(Object e, StackTrace st) {
    CallFlowLog.issue("webrtc_error", detail: e.toString());
    AppLog.instance.e("Call/WebRTC", "error", e, st);
  }
}

bool get callWebRtcSupported => !kIsWeb;

bool get callSpeakerToggleSupported =>
    !kIsWeb &&
    (defaultTargetPlatform == TargetPlatform.android ||
        defaultTargetPlatform == TargetPlatform.iOS);
