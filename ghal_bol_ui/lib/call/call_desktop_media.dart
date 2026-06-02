import "package:flutter/foundation.dart";
import "package:flutter_webrtc/flutter_webrtc.dart";
import "package:ghal_bol_ui/app_log.dart";
import "package:ghal_bol_ui/call/call_flow_log.dart";

/// Desktop call media: OS-default playback; capture without forcing BT HFP.
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

  /// No [selectAudioOutput] / [RTCVideoRenderer.audioOutput] — those change routing.
  static Future<void> bindRemoteAudioOutput(RTCVideoRenderer renderer) async {}

  static const _audioProcessingOff = {
    "echoCancellation": false,
    "noiseSuppression": false,
    "autoGainControl": false,
  };

  /// Mic: avoid BT hands-free capture (switches whole card to HFP on Linux).
  /// Speaker: always OS default via renderer [srcObject] only.
  static Future<Object> audioConstraints() async {
    if (!isDesktopNative) return true;
    try {
      final devices = await navigator.mediaDevices.enumerateDevices();
      final inputs =
          devices.where((d) => d.kind == "audioinput").toList(growable: false);
      final mic = _pickCaptureInput(inputs);
      final id = mic?.deviceId.trim() ?? "";
      if (id.isNotEmpty) {
        CallFlowLog.media("mic", {
          "route": "pinned_non_hfp",
          "label": mic!.label,
        });
        return {..._audioProcessingOff, "deviceId": id};
      }
    } catch (e, st) {
      AppLog.instance.e("Call/Media", "audioConstraints enumerate", e, st);
    }
    CallFlowLog.media("mic", {"route": "system_default"});
    return _audioProcessingOff;
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

  static MediaDeviceInfo? _pickCaptureInput(List<MediaDeviceInfo> inputs) {
    MediaDeviceInfo? defaultNonHfp;
    MediaDeviceInfo? builtIn;

    for (final d in inputs) {
      final label = d.label.trim().toLowerCase();
      if (_isHandsFreeProfile(label)) continue;
      if (label.contains("monitor") || label.contains("virtual")) continue;
      if (d.deviceId.trim().isEmpty) continue;

      if (label.startsWith("default:") || label.startsWith("default ")) {
        defaultNonHfp = d;
        break;
      }
      if (builtIn == null && _looksBuiltInMic(label)) {
        builtIn = d;
      }
    }

    // PipeWire often marks only the HFP mic as "default:" — use built-in so BT stays on A2DP.
    return defaultNonHfp ?? builtIn;
  }

  static bool _looksBuiltInMic(String label) {
    return label.contains("built-in") ||
        label.contains("builtin") ||
        label.contains("internal") ||
        label.contains("analog") ||
        label.contains("laptop") ||
        label.contains("microphone array") ||
        label.contains("pch");
  }

  static bool _isHandsFreeProfile(String label) {
    return label.contains("hands-free") ||
        label.contains("handsfree") ||
        label.contains("hfp") ||
        label.contains("headset-hf") ||
        label.contains("headset_hfp") ||
        label.contains("sco");
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
