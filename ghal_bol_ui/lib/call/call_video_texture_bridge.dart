import "dart:io";

import "package:flutter/foundation.dart";
import "package:flutter/services.dart";

/// Thin embedder bridge: register a GPU [Texture] backed by native shm RGBA.
class CallVideoTextureBridge {
  CallVideoTextureBridge._();

  static const MethodChannel _channel =
      MethodChannel("ghal_bol/call_video_texture");

  static bool get supported =>
      !kIsWeb && (Platform.isAndroid || Platform.isLinux);

  static Future<int?> register({
    required String shmPath,
    required int width,
    required int height,
  }) async {
    if (!supported) return null;
    try {
      final raw = await _channel.invokeMethod<dynamic>("register", {
        "shmPath": shmPath,
        "width": width,
        "height": height,
      });
      if (raw is int) return raw;
      if (raw is num) return raw.toInt();
      return null;
    } catch (_) {
      return null;
    }
  }

  static Future<void> release(int textureId) async {
    if (!supported) return;
    try {
      await _channel.invokeMethod<void>("release", {
        "textureId": textureId,
      });
    } catch (_) {}
  }

  static Future<void> releaseAll() async {
    if (!supported) return;
    try {
      await _channel.invokeMethod<void>("releaseAll");
    } catch (_) {}
  }
}
