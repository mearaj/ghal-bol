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

/// Dismisses the "unlock needed" notification posted by [GhalBolP2pService] after
/// device boot or START_STICKY restart.
Future<void> cancelUnlockNotification() async {
  if (!Platform.isAndroid) return;
  try {
    await _embedderChannel.invokeMethod<void>("cancelUnlockNotification");
  } catch (_) {}
}

/// Android: `true` when aggressive background battery restrictions apply (user should act).
Future<bool> isBatteryOptimized() async {
  if (!Platform.isAndroid) return false;
  try {
    final v = await _embedderChannel.invokeMethod<bool>("isBatteryOptimized");
    return v == true;
  } catch (_) {
    return false;
  }
}

Future<void> requestBatteryOptimizationExemption() async {
  if (!Platform.isAndroid) return;
  try {
    await _embedderChannel.invokeMethod<void>("requestBatteryOptimizationExemption");
  } catch (_) {}
}

/// Android 11+: `true` when "Pause app activity if unused" is enabled.
Future<bool> isUnusedAppPauseEnabled() async {
  if (!Platform.isAndroid) return false;
  try {
    final v = await _embedderChannel.invokeMethod<bool>("isUnusedAppPauseEnabled");
    return v == true;
  } catch (_) {
    return false;
  }
}

Future<void> openUnusedAppSettings() async {
  if (!Platform.isAndroid) return;
  try {
    await _embedderChannel.invokeMethod<void>("openUnusedAppSettings");
  } catch (_) {}
}

Future<List<String>> pendingNativeBackgroundSteps() async {
  if (!Platform.isAndroid) return const [];
  try {
    final raw = await _embedderChannel.invokeMethod<List<Object?>>(
      "pendingNativeBackgroundSteps",
    );
    if (raw == null) return const [];
    return raw.map((e) => e.toString()).toList();
  } catch (_) {
    return const [];
  }
}

Future<bool> needsOemBackgroundStep() async {
  if (!Platform.isAndroid) return false;
  try {
    final v = await _embedderChannel.invokeMethod<bool>("needsOemBackgroundStep");
    return v == true;
  } catch (_) {
    return false;
  }
}

Future<bool> openOemBackgroundSettings() async {
  if (!Platform.isAndroid) return false;
  try {
    final v = await _embedderChannel.invokeMethod<bool>("openOemBackgroundSettings");
    return v == true;
  } catch (_) {
    return false;
  }
}

Future<void> markOemBackgroundStepAcknowledged() async {
  if (!Platform.isAndroid) return;
  try {
    await _embedderChannel.invokeMethod<void>("markOemBackgroundStepAcknowledged");
  } catch (_) {}
}
