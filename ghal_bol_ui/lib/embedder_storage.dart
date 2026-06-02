import "dart:io";

import "package:flutter/services.dart";
import "package:ghal_bol_ui/app_log.dart";
import "package:ghal_bol_ui/ghal_bol_ffi.dart";

const MethodChannel _embedderChannel = MethodChannel("ghal_bol/embedder");

/// Align in-process FFI storage with the Android embedder (`:p2p` uses the same root).
///
/// Desktop/Linux/macOS/Windows: Rust resolves paths via `directories` — no Dart paths.
Future<void> ghalBolAlignNativeStorage() async {
  if (!GhalBolFfi.isLibraryLoaded) return;
  if (Platform.isAndroid) {
    try {
      final root = await _embedderChannel.invokeMethod<String>("dataRootForFfi");
      if (root != null && root.isNotEmpty) {
        GhalBolFfi.configureAndroidDataDirectory(root);
        AppLog.instance.i("Storage", "embedder data root configured");
      }
    } catch (e, st) {
      AppLog.instance.e("Storage", "embedder data root failed", e, st);
    }
    return;
  }
  AppLog.instance.d("Storage", "using native platform data root");
}
