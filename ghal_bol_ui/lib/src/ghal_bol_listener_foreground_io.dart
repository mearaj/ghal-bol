import "dart:io" show Platform;

import "package:flutter/services.dart";
import "package:ghal_bol_ui/app_log.dart";

const MethodChannel _channel = MethodChannel("ghal_bol/listener");

/// Android: starts the foreground service. Notification permission is requested by
/// [AndroidBackgroundReadiness] before P2P bootstrap — do not prompt here (avoids overlap).
Future<void> ghalBolListenerForegroundEnsureStarted() async {
  if (!Platform.isAndroid) return;
  try {
    await _channel.invokeMethod<void>("startForeground");
    AppLog.instance.i("Listener", "foreground service started");
  } catch (e, st) {
    AppLog.instance.e("Listener", "foreground start failed", e, st);
  }
}

Future<void> ghalBolListenerForegroundStop() async {
  if (!Platform.isAndroid) return;
  try {
    await _channel.invokeMethod<void>("stopForeground");
    AppLog.instance.i("Listener", "foreground service stopped");
  } catch (e) {
    AppLog.instance.w("Listener", "foreground stop failed: $e");
  }
}
