import "dart:async";

import "package:audioplayers/audioplayers.dart";
import "package:flutter/foundation.dart";
import "package:flutter/services.dart";

import "package:ghal_bol_ui/call/call_flow_log.dart";

enum _CallRingtoneKind { none, incoming, outgoing }

/// Looping ring / ringback tones during call setup (Linux + Android).
abstract final class CallRingtone {
  static AudioPlayer? _player;
  static _CallRingtoneKind _kind = _CallRingtoneKind.none;
  static Timer? _vibrateTimer;

  static Future<void> startIncoming() => _start(
        asset: "call/incoming_ring.wav",
        kind: _CallRingtoneKind.incoming,
        vibrate: true,
      );

  static Future<void> startOutgoing() => _start(
        asset: "call/ringback.wav",
        kind: _CallRingtoneKind.outgoing,
        vibrate: false,
      );

  static Future<void> stop() async {
    _vibrateTimer?.cancel();
    _vibrateTimer = null;
    final p = _player;
    _player = null;
    _kind = _CallRingtoneKind.none;
    if (p == null) return;
    try {
      await p.stop();
      await p.dispose();
    } catch (_) {}
  }

  static Future<void> _start({
    required String asset,
    required _CallRingtoneKind kind,
    required bool vibrate,
  }) async {
    if (_kind == kind) return;
    await stop();
    _kind = kind;
    try {
      final player = AudioPlayer();
      _player = player;
      await player.setReleaseMode(ReleaseMode.loop);
      await player.setVolume(kind == _CallRingtoneKind.incoming ? 0.9 : 0.75);
      await player.play(AssetSource(asset));
      CallFlowLog.step("ringtone", {"kind": kind.name});
      if (vibrate) _startIncomingVibration();
    } catch (e) {
      CallFlowLog.issue("ringtone_failed", detail: e.toString());
      _kind = _CallRingtoneKind.none;
      _player = null;
    }
  }

  static void _startIncomingVibration() {
    if (kIsWeb || defaultTargetPlatform != TargetPlatform.android) return;
    _vibrateTimer?.cancel();
    unawaited(HapticFeedback.heavyImpact());
    _vibrateTimer = Timer.periodic(const Duration(seconds: 2), (_) {
      unawaited(HapticFeedback.heavyImpact());
    });
  }
}
