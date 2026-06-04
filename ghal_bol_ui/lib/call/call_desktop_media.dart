import "package:flutter/foundation.dart";
import "package:flutter_webrtc/flutter_webrtc.dart";
import "package:ghal_bol_ui/app_log.dart";
import "package:ghal_bol_ui/call/call_flow_log.dart";

/// Desktop call media: OS-default playback; non-HFP capture via flutter_webrtc [sourceId].
abstract final class CallDesktopMedia {
  static String? _videoInputId;

  static bool get isDesktopNative =>
      !kIsWeb &&
      (defaultTargetPlatform == TargetPlatform.linux ||
          defaultTargetPlatform == TargetPlatform.windows ||
          defaultTargetPlatform == TargetPlatform.macOS);

  static Future<void> prepareForCall() async {
    if (!isDesktopNative) return;
    await _refreshCameraId();
  }

  static void clearCallSession() {
    _videoInputId = null;
  }

  static void logCaptureTrack(MediaStreamTrack track) {
    final label = track.label?.trim() ?? "";
    CallFlowLog.media("capture_track", {
      "label": label.isEmpty ? "(empty)" : label,
      "enabled": track.enabled.toString(),
    });
  }

  static Future<void> bindRemoteAudioOutput(RTCVideoRenderer renderer) async {}

  static const List<Map<String, dynamic>> _audioProcessingOffOptional = [
    {"echoCancellation": false},
    {"googEchoCancellation": false},
    {"googEchoCancellation2": false},
    {"googDAEchoCancellation": false},
    {"googNoiseSuppression": false},
    {"noiseSuppression": false},
    {"autoGainControl": false},
  ];

  /// Echo processing off only — no enumerate/select (fast; avoids BT "default:" HFP).
  static Map<String, dynamic> audioConstraints() {
    CallFlowLog.media("mic", {"route": "os_default_no_pin"});
    return {"optional": _audioProcessingOffOptional};
  }

  static Map<String, dynamic> get _videoSizeHints => {
        "width": {"ideal": 640, "max": 1280},
        "height": {"ideal": 480, "max": 720},
        "frameRate": {"ideal": 15, "max": 24},
      };

  static Map<String, dynamic> videoConstraints() {
    final id = _videoInputId?.trim() ?? "";
    final video = <String, dynamic>{..._videoSizeHints};
    if (id.isNotEmpty) {
      video["deviceId"] = id;
    }
    return {"video": video};
  }

  static Future<void> _refreshCameraId() async {
    if (!isDesktopNative) return;
    try {
      final devices = await navigator.mediaDevices.enumerateDevices();
      final cameras =
          devices.where((d) => d.kind == "videoinput").toList(growable: false);
      _videoInputId = _pickCamera(cameras)?.deviceId.trim();
      if (_videoInputId?.isEmpty ?? true) {
        _videoInputId = null;
      }
    } catch (e, st) {
      AppLog.instance.e("Call/Media", "refreshCameraId", e, st);
    }
  }

  static MediaDeviceInfo? _pickCamera(List<MediaDeviceInfo> list) {
    if (list.isEmpty) return null;
    int score(MediaDeviceInfo d) {
      final label = d.label.toLowerCase();
      var s = 0;
      if (label.startsWith("default:")) s += 80;
      if (label.contains("camera") ||
          label.contains("webcam") ||
          label.contains("video")) {
        s += 40;
      }
      if (label.contains("virtual") || label.contains("dummy")) s -= 200;
      return s;
    }
    var best = list.first;
    var bestScore = score(best);
    for (final d in list.skip(1)) {
      final s = score(d);
      if (s > bestScore) {
        best = d;
        bestScore = s;
      }
    }
    return best;
  }
}
