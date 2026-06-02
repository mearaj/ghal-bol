import "package:flutter/foundation.dart";
import "package:flutter/material.dart";
import "package:flutter_webrtc/flutter_webrtc.dart";

import "package:ghal_bol_ui/call/call_controller.dart";
import "package:ghal_bol_ui/call/call_webrtc.dart";

/// Full-screen call UI: voice by default, optional video toggle.
class CallScreen extends StatefulWidget {
  const CallScreen({super.key});

  @override
  State<CallScreen> createState() => _CallScreenState();
}

class _CallScreenState extends State<CallScreen> {
  final _ctrl = CallController.instance;

  @override
  void initState() {
    super.initState();
    _ctrl.callScreenVisible = true;
    _ctrl.addListener(_onChange);
  }

  @override
  void dispose() {
    _ctrl.callScreenVisible = false;
    _ctrl.removeListener(_onChange);
    super.dispose();
  }

  void _onChange() {
    if (mounted) setState(() {});
  }

  @override
  Widget build(BuildContext context) {
    final name = _ctrl.peerDisplayName ?? "Contact";
    final phase = _ctrl.phase;
    final webrtc = _ctrl.webrtc;
    final inCall = _ctrl.inCallActive;
    final showAudioSink = inCall && webrtc != null;
    final mirrorLocalPreview =
        defaultTargetPlatform == TargetPlatform.android ||
            defaultTargetPlatform == TargetPlatform.iOS;

    final status = _statusLabel(phase, _ctrl.statusMessage);
    final showSpinner = phase == CallUiPhase.outgoingRinging ||
        phase == CallUiPhase.incomingRinging;

    return Scaffold(
      backgroundColor: const Color(0xFF2D3142),
      body: SafeArea(
        child: Column(
          children: [
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
            const SizedBox(height: 24),
            Expanded(
              child: Stack(
                alignment: Alignment.center,
                children: [
                  if (showAudioSink)
                    // Voice calls need a live RTCVideoView for remote audio on desktop GTK.
                    Positioned(
                      left: 0,
                      top: 0,
                      width: _ctrl.showRemoteVideo ? null : 1,
                      height: _ctrl.showRemoteVideo ? null : 1,
                      right: _ctrl.showRemoteVideo ? 0 : null,
                      bottom: _ctrl.showRemoteVideo ? 0 : null,
                      child: IgnorePointer(
                        child: RTCVideoView(
                          webrtc.remoteRenderer,
                          key: ValueKey<String>(
                            "remote-${webrtc.remoteStream?.id ?? "none"}-"
                            "${webrtc.remoteStream?.getVideoTracks().length ?? 0}",
                          ),
                          objectFit: _ctrl.showRemoteVideo
                              ? RTCVideoViewObjectFit.RTCVideoViewObjectFitCover
                              : RTCVideoViewObjectFit.RTCVideoViewObjectFitContain,
                        ),
                      ),
                    ),
                  if (!_ctrl.showRemoteVideo) ...[
                    Icon(
                      Icons.person,
                      size: 120,
                      color: Colors.white.withValues(alpha: 0.35),
                    ),
                    if (showSpinner)
                      const Padding(
                        padding: EdgeInsets.only(top: 160),
                        child: CircularProgressIndicator(color: Colors.white54),
                      ),
                  ],
                  if (_ctrl.showLocalPreview && webrtc != null)
                    Positioned(
                      right: 16,
                      bottom: 16,
                      width: 100,
                      height: 140,
                      child: ClipRRect(
                        borderRadius: BorderRadius.circular(12),
                        child: RTCVideoView(
                          webrtc.localRenderer,
                          mirror: mirrorLocalPreview,
                          objectFit: RTCVideoViewObjectFit
                              .RTCVideoViewObjectFitContain,
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
    );
  }

  Widget _connectedControls(CallUiPhase phase) {
    final showMedia = _ctrl.inCallActive && callWebRtcSupported;
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
            icon: _ctrl.localVideoOn ? Icons.videocam_off : Icons.videocam,
            color: Colors.white24,
            tooltip: "Video",
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
