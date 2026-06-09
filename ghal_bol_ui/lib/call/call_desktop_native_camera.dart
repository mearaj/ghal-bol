import "dart:async";
import "dart:convert";

import "package:camera/camera.dart";
import "package:flutter/foundation.dart";
import "package:flutter/material.dart";

import "package:ghal_bol_ui/app_log.dart";
import "package:ghal_bol_ui/call/call_desktop_media.dart";
import "package:ghal_bol_ui/call/call_flow_log.dart";
import "package:ghal_bol_ui/ghal_bol_p2p.dart";

/// Desktop native video: Flutter owns the webcam in the UI process and pushes
/// I420 frames to the daemon for encode/send (no WebRTC).
///
/// Linux + macOS use [`camera`] + [`camera_desktop`] (GStreamer / AVFoundation,
/// PipeWire portal on Linux) with real-time YUV streaming. Windows has no
/// `camera_desktop` image stream yet, so desktop **capture** is unavailable there
/// until native Windows capture lands — calls still receive/decode the peer.
abstract final class CallDesktopNativeCamera {
  static CameraController? _cameraController;
  static String? _activeCallId;
  static bool _pushInFlight = false;
  static DateTime? _lastPush;
  static bool _loggedFirstPush = false;

  static final _listeners = <VoidCallback>[];

  static const Duration _minPushInterval = Duration(milliseconds: 33); // ~30 fps

  /// Updated after native video starts — `false` when the daemon owns nokhwa capture.
  static bool _flutterInject = true;

  static bool get usesFlutterCapture =>
      CallDesktopMedia.isDesktopNative && _flutterInject;

  /// Query Rust after `callVideoStart` (async session setup).
  static Future<void> refreshCaptureBackend() async {
    if (!CallDesktopMedia.isDesktopNative) return;
    for (var i = 0; i < 24; i++) {
      final r = await GhalBolP2p.callVideoCaptureBackend();
      final b = r["backend"]?.toString() ?? "none";
      if (b != "none") {
        _flutterInject = b == "flutter";
        CallFlowLog.step("desktop_capture_backend", {"backend": b});
        return;
      }
      await Future<void>.delayed(const Duration(milliseconds: 50));
    }
    _flutterInject = true;
  }

  static void resetCaptureBackend() {
    _flutterInject = true;
  }

  /// `camera_desktop` exposes YUV image streaming on Linux and macOS only.
  static bool get _useCameraPlugin =>
      defaultTargetPlatform == TargetPlatform.linux ||
      defaultTargetPlatform == TargetPlatform.macOS;

  static bool get isActive {
    if (!usesFlutterCapture) return false;
    return _cameraController?.value.isInitialized == true;
  }

  static void addListener(VoidCallback l) => _listeners.add(l);
  static void removeListener(VoidCallback l) => _listeners.remove(l);

  static void _notify() {
    for (final l in List<VoidCallback>.from(_listeners)) {
      l();
    }
  }

  /// Probe `camera_desktop` early (PipeWire portal permission, GStreamer init).
  /// Call when the call UI opens.
  static Future<void> warmup() async {
    if (!usesFlutterCapture || !_useCameraPlugin) return;
    try {
      final cameras = await _enumerateCameras();
      CallFlowLog.step("desktop_camera_probe", {
        "count": cameras.length.toString(),
        "platform": defaultTargetPlatform.name,
      });
    } catch (e, st) {
      AppLog.instance.e("Call/DesktopCamera", "warmup probe failed", e, st);
    }
  }

  /// Linux PipeWire portal enumeration can return empty until permission is granted.
  static Future<List<CameraDescription>> _enumerateCameras() async {
    const attempts = 6;
    const retryDelay = Duration(milliseconds: 500);
    List<CameraDescription> cameras = const [];
    for (var i = 0; i < attempts; i++) {
      cameras = await availableCameras();
      if (cameras.isNotEmpty) return cameras;
      if (i + 1 < attempts) {
        await Future<void>.delayed(retryDelay);
      }
    }
    return cameras;
  }

  static Future<void> start({required String callId}) async {
    if (!usesFlutterCapture) return;
    if (_activeCallId == callId && isActive) return;
    await stop();
    _activeCallId = callId;
    if (!_useCameraPlugin) {
      // Windows: no `camera_desktop` image stream — receive-only until native capture.
      CallFlowLog.issue(
        "desktop_camera_unsupported",
        check: "Windows native capture not implemented; video is receive-only",
      );
      return;
    }
    try {
      final cameras = await _enumerateCameras();
      if (cameras.isEmpty) {
        throw StateError(
          "no camera found — install GStreamer (Arch: pacman -S gstreamer "
          "gst-plugins-base gst-plugins-good), allow camera in PipeWire portal, "
          "and check /dev/video*",
        );
      }
      await _startCameraPlugin(callId, cameras);
      _notify();
    } catch (e, st) {
      AppLog.instance.e("Call/DesktopCamera", "start", e, st);
      CallFlowLog.issue(
        "desktop_camera_failed",
        check: "webcam / PipeWire; gstreamer gst-plugins-good (Arch: pacman -S gstreamer gst-plugins-base gst-plugins-good)",
        detail: e.toString(),
      );
      rethrow;
    }
  }

  static Future<void> stop() async {
    if (!CallDesktopMedia.isDesktopNative) return;
    _activeCallId = null;
    _pushInFlight = false;
    _loggedFirstPush = false;
    _lastPush = null;

    final cam = _cameraController;
    _cameraController = null;
    if (cam != null) {
      try {
        if (cam.value.isStreamingImages) {
          await cam.stopImageStream();
        }
      } catch (_) {}
      try {
        await cam.dispose();
      } catch (_) {}
    }
    _notify();
  }

  static Future<void> _startCameraPlugin(
    String callId,
    List<CameraDescription> cameras,
  ) async {
    final ctrl = CameraController(
      _pickCamera(cameras),
      ResolutionPreset.medium,
      enableAudio: false,
      fps: 30,
    );
    await ctrl.initialize();
    _cameraController = ctrl;
    await ctrl.startImageStream((image) => _onCameraImage(callId, image));
    CallFlowLog.step("desktop_camera_start", {
      "route": "camera_desktop",
      "platform": defaultTargetPlatform.name,
    });
  }

  static CameraDescription _pickCamera(List<CameraDescription> list) {
    for (final c in list) {
      if (c.lensDirection == CameraLensDirection.front) return c;
    }
    CameraDescription? best;
    var bestScore = -999;
    for (final c in list) {
      final l = c.name.toLowerCase();
      var score = 0;
      if (l.contains("virtual") || l.contains("dummy")) score -= 200;
      if (l.contains("camera") || l.contains("webcam") || l.contains("video")) {
        score += 40;
      }
      if (score > bestScore) {
        best = c;
        bestScore = score;
      }
    }
    return best ?? list.first;
  }

  static void _onCameraImage(String callId, CameraImage image) {
    if (_activeCallId != callId || _pushInFlight) return;
    final last = _lastPush;
    final now = DateTime.now();
    if (last != null && now.difference(last) < _minPushInterval) return;
    _lastPush = now;
    _pushInFlight = true;
    unawaited(_pushCameraImage(callId, image));
  }

  static Future<void> _pushCameraImage(String callId, CameraImage image) async {
    try {
      // Packed BGRA/RGBA (the desktop `camera_desktop` path): send raw pixels +
      // stride and let Rust pack to I420 — no per-pixel Dart loop on the UI isolate.
      if (image.format.group == ImageFormatGroup.bgra8888 &&
          image.planes.length == 1) {
        final plane = image.planes[0];
        final w = image.width & ~1;
        final h = image.height & ~1;
        if (w <= 0 || h <= 0) return;
        final r = await GhalBolP2p.callVideoPushCameraFrame(
          callId: callId,
          width: w,
          height: h,
          dataBase64: base64Encode(plane.bytes),
          format: _isRgbaPixels(image) ? "rgba" : "bgra",
          stride: plane.bytesPerRow,
        );
        if (r["ok"] == true && !_loggedFirstPush) {
          _loggedFirstPush = true;
          CallFlowLog.step("desktop_camera_push", {
            "w": w.toString(),
            "h": h.toString(),
            "format": "packed_${_isRgbaPixels(image) ? "rgba" : "bgra"}",
            "raw": image.format.raw?.toString() ?? "?",
          });
        }
        return;
      }
      // Rare planar YUV420 desktop source — convert in Dart and send I420.
      final converted = _yuv420ToI420(image);
      if (converted == null) return;
      final (i420, w, h) = converted;
      final r = await GhalBolP2p.callVideoPushCameraFrame(
        callId: callId,
        width: w,
        height: h,
        dataBase64: base64Encode(i420),
        format: "i420",
      );
      if (r["ok"] == true && !_loggedFirstPush) {
        _loggedFirstPush = true;
        CallFlowLog.step("desktop_camera_push", {
          "w": w.toString(),
          "h": h.toString(),
          "format": image.format.group.name,
        });
      }
    } catch (e, st) {
      AppLog.instance.e("Call/DesktopCamera", "pushFrame", e, st);
    } finally {
      _pushInFlight = false;
    }
  }

  /// `camera_desktop` uses [ImageFormatGroup.bgra8888] for both BGRA (macOS) and
  /// RGBA (Linux GStreamer) — check [CameraImageFormat.raw].
  static bool _isRgbaPixels(CameraImage image) {
    final raw = image.format.raw?.toString().toUpperCase() ?? "";
    if (raw.contains("RGBA")) return true;
    if (raw.contains("BGRA")) return false;
    return defaultTargetPlatform == TargetPlatform.linux;
  }

  /// Planar I420 (Y + U + V) from a YUV420 camera frame.
  static (Uint8List, int, int)? _yuv420ToI420(CameraImage image) {
    final w = image.width & ~1;
    final h = image.height & ~1;
    if (w <= 0 || h <= 0 || image.planes.length < 3) return null;

    final yPlane = image.planes[0];
    final uPlane = image.planes[1];
    final vPlane = image.planes[2];
    final ySize = w * h;
    final uvW = w ~/ 2;
    final uvH = h ~/ 2;
    final out = Uint8List(ySize + 2 * uvW * uvH);

    var yo = 0;
    for (var row = 0; row < h; row++) {
      final src = row * yPlane.bytesPerRow;
      out.setRange(yo, yo + w, yPlane.bytes, src);
      yo += w;
    }
    var uo = ySize;
    var vo = ySize + uvW * uvH;
    final uStep = uPlane.bytesPerPixel ?? 1;
    final vStep = vPlane.bytesPerPixel ?? 1;
    for (var row = 0; row < uvH; row++) {
      final uSrc = row * uPlane.bytesPerRow;
      final vSrc = row * vPlane.bytesPerRow;
      for (var col = 0; col < uvW; col++) {
        out[uo++] = uPlane.bytes[uSrc + col * uStep];
        out[vo++] = vPlane.bytes[vSrc + col * vStep];
      }
    }
    return (out, w, h);
  }

  /// PiP preview — `camera_desktop` on Linux/macOS (no preview on Windows yet).
  static Widget buildPiP() {
    final ctrl = _cameraController;
    if (ctrl != null && ctrl.value.isInitialized) {
      return FittedBox(
        fit: BoxFit.cover,
        child: CameraPreview(ctrl),
      );
    }
    return const ColoredBox(color: Color(0xFF1A1D26));
  }
}
