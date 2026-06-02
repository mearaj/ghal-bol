import "dart:io" show Platform;

import "package:flutter/services.dart";
import "package:ghal_bol_ui/app_log.dart";
import "package:permission_handler/permission_handler.dart";

const MethodChannel _channel = MethodChannel("ghal_bol/listener");

bool _androidForegroundStarted = false;

/// Android: requests notification permission when needed, then starts the foreground
/// service so the OS is less likely to kill the process while the libp2p listener runs.
Future<void> ghalBolListenerForegroundEnsureStarted() async {
  if (!Platform.isAndroid) return;
  if (!_androidForegroundStarted) {
    final status = await Permission.notification.status;
    if (!status.isGranted) {
      await Permission.notification.request();
    }
  }
  try {
    await _channel.invokeMethod<void>("startForeground");
    _androidForegroundStarted = true;
    AppLog.instance.i("Listener", "foreground service started");
  } catch (e, st) {
    AppLog.instance.e("Listener", "foreground start failed", e, st);
  }
}

Future<void> ghalBolListenerForegroundStop() async {
  if (!Platform.isAndroid) return;
  try {
    await _channel.invokeMethod<void>("stopForeground");
    _androidForegroundStarted = false;
    AppLog.instance.i("Listener", "foreground service stopped");
  } catch (e) {
    AppLog.instance.w("Listener", "foreground stop failed: $e");
  }
}
