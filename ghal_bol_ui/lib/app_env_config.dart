import "dart:io";

import "package:flutter/foundation.dart";
import "package:flutter/services.dart";

import "app_log.dart";

/// Loads bundled `env/.env.development` (debug) or `env/.env.production` (release).
///
/// [get] checks [Platform.environment] first, then values from that file.
abstract final class AppEnvConfig {
  static final Map<String, String> _fromFile = {};
  static bool _loaded = false;

  static String get _envAssetPath =>
      kDebugMode ? "env/.env.development" : "env/.env.production";

  static Future<void> load() async {
    if (_loaded) return;
    _loaded = true;

    final path = _envAssetPath;

    // Desktop debug: read live repo file first — `flutter run` keeps a stale bundled
    // copy until rebuild; editing env/.env.development must apply without that trap.
    if (kDebugMode && !Platform.isAndroid && !Platform.isIOS) {
      for (final filePath in [path, "ghal_bol_ui/$path"]) {
        final f = File(filePath);
        if (!await f.exists()) continue;
        final diskRaw = await f.readAsString();
        _parseEnvFile(diskRaw);
        if (_fromFile.isNotEmpty) {
          AppLog.instance.i(
            "Env",
            "loaded ${f.absolute.path} (${_fromFile.length} keys)",
          );
          await _warnIfBundledCoordUrlDiffers(path, diskRaw);
          _logCoordUrlsFromEnv();
          return;
        }
      }
    }

    try {
      final raw = await rootBundle.loadString(path);
      _parseEnvFile(raw);
      if (_fromFile.isNotEmpty) {
        AppLog.instance.i("Env", "loaded asset $path (${_fromFile.length} keys)");
        _logCoordUrlsFromEnv();
        return;
      }
    } catch (_) {}

    // Mobile / release fallback when bundled asset missing.
    if (!Platform.isAndroid && !Platform.isIOS) {
      for (final filePath in [path, "ghal_bol_ui/$path"]) {
        final f = File(filePath);
        if (!await f.exists()) continue;
        _parseEnvFile(await f.readAsString());
        if (_fromFile.isNotEmpty) {
          AppLog.instance.i(
            "Env",
            "loaded ${f.absolute.path} (${_fromFile.length} keys)",
          );
          _logCoordUrlsFromEnv();
          return;
        }
      }
    }

    AppLog.instance.w(
      "Env",
      "missing $path — set GHAL_BOL_COORD_URLS in that file (see env/README.md)",
    );
  }

  static void _logCoordUrlsFromEnv() {
    const key = "GHAL_BOL_COORD_URLS";
    final shell = Platform.environment[key]?.trim();
    if (shell != null && shell.isNotEmpty) {
      AppLog.instance.i("Env", "$key from OS env: $shell");
      return;
    }
    final file = _fromFile[key];
    if (file != null && file.isNotEmpty) {
      AppLog.instance.i("Env", "$key from file: $file");
    }
  }

  /// Bundled asset can lag behind disk on desktop; Android always uses the bundle.
  static Future<void> _warnIfBundledCoordUrlDiffers(
    String assetPath,
    String diskRaw,
  ) async {
    if (!kDebugMode) return;
    const key = "GHAL_BOL_COORD_URLS";
    final diskVal = _peekEnvValue(diskRaw, key);
    if (diskVal == null || diskVal.isEmpty) return;
    try {
      final bundleRaw = await rootBundle.loadString(assetPath);
      final bundleVal = _peekEnvValue(bundleRaw, key);
      if (bundleVal != null &&
          bundleVal.isNotEmpty &&
          bundleVal != diskVal) {
        AppLog.instance.w(
          "Env",
          "bundled $key differs from disk ($bundleVal vs $diskVal) — "
          "Linux uses disk; rebuild app for Android",
        );
      }
    } catch (_) {}
  }

  static String? _peekEnvValue(String raw, String wantKey) {
    for (final line in raw.split("\n")) {
      var s = line.trim();
      if (s.isEmpty || s.startsWith("#")) continue;
      if (s.startsWith("export ")) s = s.substring(7).trim();
      final eq = s.indexOf("=");
      if (eq <= 0) continue;
      if (s.substring(0, eq).trim() != wantKey) continue;
      var val = s.substring(eq + 1).trim();
      if ((val.startsWith('"') && val.endsWith('"')) ||
          (val.startsWith("'") && val.endsWith("'"))) {
        val = val.substring(1, val.length - 1);
      }
      return val;
    }
    return null;
  }

  static void _parseEnvFile(String raw) {
    _fromFile.clear();
    for (final line in raw.split("\n")) {
      var s = line.trim();
      if (s.isEmpty || s.startsWith("#")) continue;
      if (s.startsWith("export ")) {
        s = s.substring(7).trim();
      }
      final eq = s.indexOf("=");
      if (eq <= 0) continue;
      final key = s.substring(0, eq).trim();
      var val = s.substring(eq + 1).trim();
      if ((val.startsWith('"') && val.endsWith('"')) ||
          (val.startsWith("'") && val.endsWith("'"))) {
        val = val.substring(1, val.length - 1);
      }
      if (key.isEmpty) continue;
      if (Platform.environment.containsKey(key)) continue;
      _fromFile[key] = val;
    }
  }

  /// OS env wins; otherwise value from bundled / parsed `.env.*`.
  static String? get(String key) {
    final k = key.trim();
    if (k.isEmpty) return null;
    final shell = Platform.environment[k]?.trim();
    if (shell != null && shell.isNotEmpty) return shell;
    return _fromFile[k];
  }
}
