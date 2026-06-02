import "dart:io";

import "package:flutter/services.dart";
import "package:ghal_bol_ui/app_log.dart";

const MethodChannel _p2pChannel = MethodChannel("ghal_bol/p2p_daemon");

/// Starts Android `:p2p` foreground service; returns Unix socket path for JSON-RPC.
Future<String> ghalBolAndroidStartP2pService() async {
  if (!Platform.isAndroid) {
    throw UnsupportedError("P2P service only on Android");
  }
  final path = await _p2pChannel.invokeMethod<String>("startP2pService");
  if (path == null || path.isEmpty) {
    throw StateError("startP2pService returned no socket path");
  }
  AppLog.instance.i("P2P", "android service socket=$path");
  return path;
}

Future<String> ghalBolAndroidP2pSocketPath() async {
  if (!Platform.isAndroid) {
    throw UnsupportedError("P2P service only on Android");
  }
  final path = await _p2pChannel.invokeMethod<String>("getSocketPath");
  return path ?? "";
}

Future<void> ghalBolAndroidStopP2pService() async {
  if (!Platform.isAndroid) return;
  await _p2pChannel.invokeMethod<void>("stopP2pService");
}
