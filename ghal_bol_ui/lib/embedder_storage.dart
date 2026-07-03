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

/// Android 6+: returns `true` when the app is subject to battery optimization (not whitelisted).
/// Once whitelisted via [requestBatteryOptimizationExemption], returns `false` permanently
/// unless the user revokes it manually in settings.
Future<bool> isBatteryOptimized() async {
  if (!Platform.isAndroid) return false;
  try {
    final v = await _embedderChannel.invokeMethod<bool>("isBatteryOptimized");
    return v == true;
  } catch (_) {
    return false;
  }
}

/// Shows the standard Android system dialog: "Allow [app] to always run in the background?"
/// Once allowed, the app is permanently whitelisted for battery optimization.
Future<void> requestBatteryOptimizationExemption() async {
  if (!Platform.isAndroid) return;
  try {
    await _embedderChannel.invokeMethod<void>("requestBatteryOptimizationExemption");
  } catch (_) {}
}

/// Android 11+: returns `true` when "Pause app activity if unused" is enabled for this app.
/// On non-Android platforms or older API levels, returns `false`.
Future<bool> isUnusedAppPauseEnabled() async {
  if (!Platform.isAndroid) return false;
  try {
    final v = await _embedderChannel.invokeMethod<bool>("isUnusedAppPauseEnabled");
    return v == true;
  } catch (_) {
    return false;
  }
}

/// Opens the Android app settings page where the user can toggle off
/// "Pause app activity if unused".
Future<void> openUnusedAppSettings() async {
  if (!Platform.isAndroid) return;
  try {
    await _embedderChannel.invokeMethod<void>("openUnusedAppSettings");
  } catch (_) {}
}
