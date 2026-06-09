import "dart:async";

import "package:flutter/foundation.dart";
import "package:flutter/material.dart";

import "package:ghal_bol_ui/call/call_controller.dart";
import "package:ghal_bol_ui/call/call_desktop_media.dart";
import "package:ghal_bol_ui/call/call_desktop_native_camera.dart";
import "package:ghal_bol_ui/call/call_native_video.dart";

/// Full-screen call UI: voice by default, optional video toggle.
class CallScreen extends StatefulWidget {
  const CallScreen({super.key});

  @override
  State<CallScreen> createState() => _CallScreenState();
}

class _CallScreenState extends State<CallScreen> with SingleTickerProviderStateMixin {
  final _ctrl = CallController.instance;
  late final AnimationController _ringPulse;
  bool _remoteFrameReady = false;
  /// Preserve texture state when PiP/main z-order swaps (no remount).
  final GlobalKey _remoteVideoKey = GlobalKey();
  final GlobalKey _localVideoKey = GlobalKey();

  @override
  void initState() {
    super.initState();
    _ringPulse = AnimationController(
      vsync: this,
      duration: const Duration(milliseconds: 1400),
    )..repeat(reverse: true);
    _ctrl.callScreenVisible = true;
    _ctrl.addListener(_onChange);
    CallDesktopNativeCamera.addListener(_onChange);
    if (CallDesktopNativeCamera.usesFlutterCapture && !_ctrl.callRestoredFromNative) {
      unawaited(CallDesktopNativeCamera.warmup());
    }
  }

  @override
  void dispose() {
    _ringPulse.dispose();
    _ctrl.callScreenVisible = false;
    _ctrl.removeListener(_onChange);
    CallDesktopNativeCamera.removeListener(_onChange);
    unawaited(_ctrl.onCallScreenDismissedWhileLive());
    super.dispose();
  }

  void _onChange() {
    if (!_ctrl.showRemoteVideo) {
      _remoteFrameReady = false;
    }
    if (mounted) setState(() {});
  }

  void _toggleVideoSwap() {
    final nativeVideo = _ctrl.nativeVideoInCall;
    final callId = _ctrl.callId ?? "";
    final remoteOn =
        nativeVideo && _ctrl.showRemoteVideo && callId.isNotEmpty;
    final localOn =
        nativeVideo && _ctrl.showLocalPreview && callId.isNotEmpty;
    _ctrl.toggleVideoMainLocal(remoteOn: remoteOn, localOn: localOn);
  }

  @override
  Widget build(BuildContext context) {
    final name = _ctrl.peerDisplayName ?? "Contact";
    final phase = _ctrl.phase;
    final inCall = _ctrl.inCallActive;
    final nativeVideo = _ctrl.nativeVideoInCall;
    final callId = _ctrl.callId ?? "";
    final showNativeRemote = nativeVideo && _ctrl.showRemoteVideo && callId.isNotEmpty;
    final showNativeLocal =
        nativeVideo && _ctrl.showLocalPreview && callId.isNotEmpty;
    final canSwapNativeVideo = showNativeRemote && showNativeLocal;
    final singleNativeVideo = showNativeRemote != showNativeLocal;
    final canTapPipToMain = canSwapNativeVideo || singleNativeVideo;
    final mainNativeTrack = switch ((showNativeRemote, showNativeLocal)) {
      (true, true) => _ctrl.videoMainShowsLocal ? "local" : "remote",
      (false, true) => "local",
      (true, false) => "remote",
      _ => "remote",
    };
    final videoCallLayout = inCall && (showNativeRemote || showNativeLocal);
    final mirrorLocalPreview =
        defaultTargetPlatform == TargetPlatform.android ||
            defaultTargetPlatform == TargetPlatform.iOS;
    // Front camera sensor is landscape; portrait UI needs one quarter-turn (local only).
    final localPreviewQuarterTurns =
        defaultTargetPlatform == TargetPlatform.android ? 1 : 0;

    final status = _statusLabel(phase, _ctrl.statusMessage);
    final showSpinner = phase == CallUiPhase.outgoingRinging ||
        phase == CallUiPhase.incomingRinging;
    final ringPulse = showSpinner
        ? Tween<double>(begin: 0.92, end: 1.08).animate(
            CurvedAnimation(parent: _ringPulse, curve: Curves.easeInOut),
          )
        : null;

    final blockBackDuringLiveCall =
        inCall && (phase == CallUiPhase.connected || phase == CallUiPhase.connecting);

    return PopScope(
      canPop: !blockBackDuringLiveCall,
      child: Scaffold(
      backgroundColor: const Color(0xFF0B0D12),
      body: SafeArea(
        child: Column(
          children: [
            if (!videoCallLayout) ...[
              const SizedBox(height: 24),
              Text(
                name,
                style: Theme.of(context).textTheme.headlineSmall?.copyWith(
                      color: Colors.white,
                      fontWeight: FontWeight.w600,
                    ),
              ),
              const SizedBox(height: 8),
              Text(
                status,
                textAlign: TextAlign.center,
                style: Theme.of(context).textTheme.bodyLarge?.copyWith(color: Colors.white70),
              ),
              if (_ctrl.callMediaE2eeLabel != null) ...[
                const SizedBox(height: 10),
                _e2eeBadge(context, _ctrl.callMediaE2eeLabel!),
              ],
              const SizedBox(height: 24),
            ],
            Expanded(
              child: Stack(
                alignment: Alignment.center,
                fit: StackFit.expand,
                children: [
                  ..._nativeVideoLayers(
                    callId: callId,
                    mainNativeTrack: mainNativeTrack,
                    showNativeRemote: showNativeRemote,
                    showNativeLocal: showNativeLocal,
                    nativeVideo: nativeVideo,
                    canTapPipToMain: canTapPipToMain,
                    mirrorLocalPreview: mirrorLocalPreview,
                    localPreviewQuarterTurns: localPreviewQuarterTurns,
                  ),
                  if (!videoCallLayout && !_ctrl.showRemoteVideo) ...[
                    if (ringPulse != null)
                      ScaleTransition(
                        scale: ringPulse,
                        child: _avatarPlaceholder(phase),
                      )
                    else
                      _avatarPlaceholder(phase),
                    if (showSpinner)
                      Padding(
                        padding: const EdgeInsets.only(top: 24),
                        child: Text(
                          phase == CallUiPhase.incomingRinging
                              ? "Ringing…"
                              : "Waiting for answer…",
                          style: Theme.of(context).textTheme.bodyMedium?.copyWith(
                                color: Colors.white54,
                              ),
                        ),
                      ),
                  ],
                  if (showNativeRemote &&
                      mainNativeTrack == "remote" &&
                      !_remoteFrameReady)
                    const Center(
                      child: Column(
                        mainAxisSize: MainAxisSize.min,
                        children: [
                          CircularProgressIndicator(color: Colors.white54, strokeWidth: 2),
                          SizedBox(height: 12),
                          Text(
                            "Connecting video…",
                            style: TextStyle(color: Colors.white54),
                          ),
                        ],
                      ),
                    ),
                  if (videoCallLayout)
                    Positioned(
                      left: 0,
                      right: 0,
                      top: 0,
                      child: DecoratedBox(
                        decoration: BoxDecoration(
                          gradient: LinearGradient(
                            begin: Alignment.topCenter,
                            end: Alignment.bottomCenter,
                            colors: [
                              Colors.black.withValues(alpha: 0.72),
                              Colors.black.withValues(alpha: 0.0),
                            ],
                          ),
                        ),
                        child: Padding(
                          padding: const EdgeInsets.fromLTRB(16, 12, 16, 28),
                          child: Column(
                            crossAxisAlignment: CrossAxisAlignment.start,
                            children: [
                              Text(
                                name,
                                style: Theme.of(context).textTheme.titleLarge?.copyWith(
                                      color: Colors.white,
                                      fontWeight: FontWeight.w600,
                                    ),
                              ),
                              if (status.isNotEmpty)
                                Text(
                                  status,
                                  style: Theme.of(context).textTheme.bodyMedium?.copyWith(
                                        color: _ctrl.callRestoredFromNative &&
                                                _ctrl.localVideoOn
                                            ? Colors.orangeAccent
                                            : Colors.white70,
                                        fontWeight: _ctrl.callRestoredFromNative &&
                                                _ctrl.localVideoOn
                                            ? FontWeight.w600
                                            : null,
                                      ),
                                ),
                            ],
                          ),
                        ),
                      ),
                    ),
                ],
              ),
            ),
            Padding(
              padding: const EdgeInsets.fromLTRB(12, 8, 12, 28),
              child: phase == CallUiPhase.incomingRinging
                  ? Row(
                      mainAxisAlignment: MainAxisAlignment.spaceEvenly,
                      children: [
                        _roundBtn(
                          icon: Icons.call_end,
                          color: Colors.red,
                          onTap: () => _ctrl.rejectIncoming(),
                        ),
                        _roundBtn(
                          icon: Icons.call,
                          color: Colors.green,
                          onTap: () => _ctrl.acceptIncoming(),
                        ),
                      ],
                    )
                  : _connectedControls(phase),
            ),
          ],
        ),
      ),
    ),
    );
  }

  /// One widget per track (stable keys); stack order puts PiP in front without remount.
  List<Widget> _nativeVideoLayers({
    required String callId,
    required String mainNativeTrack,
    required bool showNativeRemote,
    required bool showNativeLocal,
    required bool nativeVideo,
    required bool canTapPipToMain,
    required bool mirrorLocalPreview,
    required int localPreviewQuarterTurns,
  }) {
    Widget? remote;
    Widget? local;
    if (showNativeRemote) {
      remote = _positionedNativeVideo(
        callId: callId,
        track: "remote",
        isMain: mainNativeTrack == "remote",
        active: nativeVideo,
        canSwap: canTapPipToMain,
        mirrorLocalPreview: mirrorLocalPreview,
        localPreviewQuarterTurns: localPreviewQuarterTurns,
        viewKey: _remoteVideoKey,
        onFrameReady: mainNativeTrack == "remote"
            ? () {
                if (!_remoteFrameReady && mounted) {
                  setState(() => _remoteFrameReady = true);
                }
              }
            : null,
      );
    }
    if (showNativeLocal) {
      local = _positionedNativeVideo(
        callId: callId,
        track: "local",
        isMain: mainNativeTrack == "local",
        active: nativeVideo,
        canSwap: canTapPipToMain,
        mirrorLocalPreview: mirrorLocalPreview,
        localPreviewQuarterTurns: localPreviewQuarterTurns,
        viewKey: _localVideoKey,
      );
    }
    if (remote == null && local == null) return const [];
    if (remote == null) return [local!];
    if (local == null) return [remote];
    // Main under PiP — reorder only; same NativeCallVideoView keys keep textures alive.
    if (mainNativeTrack == "remote") {
      return [remote, local];
    }
    return [local, remote];
  }

  Widget _positionedNativeVideo({
    required String callId,
    required String track,
    required bool isMain,
    required bool active,
    required bool canSwap,
    required bool mirrorLocalPreview,
    required int localPreviewQuarterTurns,
    GlobalKey? viewKey,
    VoidCallback? onFrameReady,
  }) {
    final tile = _nativeVideoTile(
      callId: callId,
      track: track,
      active: active,
      fullscreen: isMain,
      mirrorLocalPreview: mirrorLocalPreview,
      localPreviewQuarterTurns: localPreviewQuarterTurns,
      viewKey: viewKey,
      onFrameReady: onFrameReady,
    );
    return Positioned(
      left: isMain ? 0 : null,
      right: isMain ? 0 : 16,
      top: isMain ? 0 : null,
      bottom: isMain ? 0 : 100,
      width: isMain ? null : 112,
      height: isMain ? null : 156,
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: !isMain && canSwap ? _toggleVideoSwap : null,
        child: isMain
            ? IgnorePointer(child: tile)
            : DecoratedBox(
                decoration: BoxDecoration(
                  borderRadius: BorderRadius.circular(14),
                  boxShadow: [
                    BoxShadow(
                      color: Colors.black.withValues(alpha: 0.45),
                      blurRadius: 12,
                      offset: const Offset(0, 4),
                    ),
                  ],
                  border: Border.all(
                    color: Colors.white.withValues(alpha: 0.85),
                    width: 2,
                  ),
                ),
                child: ClipRRect(
                  borderRadius: BorderRadius.circular(12),
                  child: tile,
                ),
              ),
      ),
    );
  }

  Widget _nativeVideoTile({
    required String callId,
    required String track,
    required bool active,
    required bool fullscreen,
    required bool mirrorLocalPreview,
    required int localPreviewQuarterTurns,
    GlobalKey? viewKey,
    VoidCallback? onFrameReady,
  }) {
    final isLocal = track == "local";
    // Windows: no `camera_desktop` image stream — PiP placeholder until native capture.
    if (isLocal &&
        !fullscreen &&
        CallDesktopNativeCamera.usesFlutterCapture &&
        defaultTargetPlatform == TargetPlatform.windows) {
      return CallDesktopNativeCamera.buildPiP();
    }
    // Key by track so PiP swap moves state with the video source (not the slot).
    // Without this, swapping reuses the wrong buffer/rotation and freezes until
    // the new track catches up (~seconds on Android).
    return NativeCallVideoView(
      key: viewKey ?? ValueKey<String>("native-video-$callId-$track"),
      callId: callId,
      track: track,
      active: active,
      mirror: isLocal && mirrorLocalPreview,
      quarterTurns: isLocal ? localPreviewQuarterTurns : 0,
      autoRotateLandscape: !isLocal && CallDesktopMedia.isDesktopNative,
      objectFit: fullscreen ? BoxFit.contain : BoxFit.cover,
      onFrameReady: onFrameReady,
    );
  }

  Widget _e2eeBadge(BuildContext context, String label) {
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 20),
      child: DecoratedBox(
        decoration: BoxDecoration(
          color: Colors.green.withValues(alpha: 0.14),
          borderRadius: BorderRadius.circular(20),
          border: Border.all(color: Colors.greenAccent.withValues(alpha: 0.45)),
        ),
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 6),
          child: Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              Icon(Icons.lock_outline, size: 16, color: Colors.greenAccent.shade100),
              const SizedBox(width: 6),
              Flexible(
                child: Text(
                  label,
                  textAlign: TextAlign.center,
                  style: Theme.of(context).textTheme.labelMedium?.copyWith(
                        color: Colors.greenAccent.shade100,
                        fontWeight: FontWeight.w500,
                      ),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }

  Widget _avatarPlaceholder(CallUiPhase phase) {
    final incoming = phase == CallUiPhase.incomingRinging;
    return Container(
      width: 132,
      height: 132,
      decoration: BoxDecoration(
        shape: BoxShape.circle,
        color: incoming
            ? Colors.green.withValues(alpha: 0.18)
            : Colors.white.withValues(alpha: 0.08),
        border: Border.all(
          color: incoming
              ? Colors.green.withValues(alpha: 0.55)
              : Colors.white.withValues(alpha: 0.2),
          width: 2,
        ),
      ),
      child: Icon(
        incoming ? Icons.phone_in_talk : Icons.person,
        size: 72,
        color: Colors.white.withValues(alpha: incoming ? 0.85 : 0.35),
      ),
    );
  }

  Widget _connectedControls(CallUiPhase phase) {
    final showMedia = _ctrl.inCallActive && _ctrl.nativeVoiceInCall;
    return Row(
      mainAxisAlignment: MainAxisAlignment.spaceEvenly,
      children: [
        if (showMedia) ...[
          _roundBtn(
            icon: _ctrl.micMuted ? Icons.mic_off : Icons.mic,
            color: _ctrl.micMuted ? Colors.orange : Colors.white24,
            tooltip: _ctrl.micMuted ? "Unmute" : "Mute",
            onTap: () => _ctrl.toggleMute(),
          ),
          if (callSpeakerToggleSupported)
            _roundBtn(
              icon: _ctrl.speakerOn ? Icons.volume_up : Icons.hearing,
              color: Colors.white24,
              tooltip: _ctrl.speakerOn ? "Speaker" : "Earpiece",
              onTap: () => _ctrl.toggleSpeaker(),
            ),
          _roundBtn(
            icon: _ctrl.onHold ? Icons.play_arrow : Icons.pause,
            color: _ctrl.onHold ? Colors.orange : Colors.white24,
            tooltip: _ctrl.onHold ? "Resume" : "Hold",
            onTap: () => _ctrl.toggleHold(),
          ),
          _roundBtn(
            icon: _ctrl.localVideoOn ? Icons.videocam : Icons.videocam_off,
            color: _ctrl.localVideoOn ? Colors.green.shade700 : Colors.white24,
            tooltip: _ctrl.localVideoOn ? "Turn video off" : "Turn video on",
            onTap: () => _ctrl.toggleVideo(),
          ),
        ],
        _roundBtn(
          icon: Icons.call_end,
          color: Colors.red,
          tooltip: "End call",
          onTap: () => _ctrl.hangUp(),
        ),
      ],
    );
  }

  String _statusLabel(CallUiPhase phase, String? custom) {
    if (custom != null && custom.isNotEmpty) return custom;
    switch (phase) {
      case CallUiPhase.outgoingRinging:
        return "Ringing…";
      case CallUiPhase.incomingRinging:
        return "Incoming call";
      case CallUiPhase.connecting:
        return "Connecting audio…";
      case CallUiPhase.connected:
        if (_ctrl.onHold) return "On hold";
        if (_ctrl.micMuted) return "Muted";
        if (_ctrl.showRemoteVideo && _ctrl.showLocalPreview) return "Video call";
        if (_ctrl.showRemoteVideo) return "Receiving video";
        if (_ctrl.showLocalPreview) return "Sending video";
        return "Voice call";
      case CallUiPhase.ended:
        return "Ended";
      case CallUiPhase.idle:
        return "";
    }
  }

  Widget _roundBtn({
    required IconData icon,
    required Color color,
    Color fg = Colors.white,
    required VoidCallback onTap,
    String? tooltip,
  }) {
    final btn = Material(
      color: color,
      shape: const CircleBorder(),
      child: InkWell(
        customBorder: const CircleBorder(),
        onTap: onTap,
        child: SizedBox(
          width: 56,
          height: 56,
          child: Icon(icon, color: fg, size: 26),
        ),
      ),
    );
    if (tooltip == null) return btn;
    return Tooltip(message: tooltip, child: btn);
  }
}
