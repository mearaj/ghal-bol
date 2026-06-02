import "dart:io";

import "package:flutter/foundation.dart";
import "package:flutter/services.dart";

import "app_log.dart";

/// Loads `env/.env.*` bundled in the APK/IPA/desktop bundle (see `pubspec.yaml` `env/`).
///
/// [get] checks [Platform.environment] first, then values parsed from those files.
/// Works on **Android, iOS, Linux, macOS, Windows** — not only desktop repo paths.
abstract final class AppEnvConfig {
  static final Map<String, String> _fromFile = {};
  static bool _loaded = false;

  static Future<void> load() async {
    if (_loaded) return;
    _loaded = true;

    final assetPaths = kDebugMode
        ? <String>[
            "env/.env.development",
            "env/.env.development.example",
            "env/.env.production",
            "env/.env.production.example",
          ]
        : <String>[
            "env/.env.production",
            "env/.env.production.example",
            "env/.env.development",
            "env/.env.development.example",
          ];

    for (final path in assetPaths) {
      try {
        final raw = await rootBundle.loadString(path);
        _parseEnvFile(raw, merge: true);
        if (_fromFile.isNotEmpty) {
          AppLog.instance.i("Env", "loaded asset $path (${_fromFile.length} keys)");
          return;
        }
      } catch (_) {
        continue;
      }
    }

    // `flutter run` on desktop: read repo files when not already bundled.
    if (!Platform.isAndroid && !Platform.isIOS) {
      final filePaths = kDebugMode
          ? <String>["env/.env.development", "ghal_bol_ui/env/.env.development"]
          : <String>["env/.env.production", "ghal_bol_ui/env/.env.production"];
      for (final path in filePaths) {
        final f = File(path);
        if (!await f.exists()) continue;
        _parseEnvFile(await f.readAsString(), merge: true);
        if (_fromFile.isNotEmpty) {
          AppLog.instance.i("Env", "loaded ${f.absolute.path} (${_fromFile.length} keys)");
          return;
        }
      }
    }

    AppLog.instance.d(
      "Env",
      "no env file in bundle — add env/.env.development (see env/README.md), "
      "or pass --dart-define=GHAL_BOL_COORD_URL=…",
    );
  }

  static void _parseEnvFile(String raw, {bool merge = false}) {
    if (!merge) _fromFile.clear();
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
