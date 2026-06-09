import "dart:convert";
import "dart:io" show Platform;

import "package:flutter/foundation.dart" show visibleForTesting;
import "package:ghal_bol_ui/app_env_config.dart";
import "package:ghal_bol_ui/ghal_bol_ffi.dart";
import "package:ghal_bol_ui/ghal_bol_constants.dart";

/// Coordination server base URLs (presence + endpoint lookup).
///
/// Resolved from `--dart-define`, OS env, bundled `env/.env.*` ([AppEnvConfig]),
/// then native preferences. Set `GHAL_BOL_COORD_URLS` in `env/.env.development` or
/// `env/.env.production` (see `env/README.md`).
abstract final class CoordinationUrl {
  static const _urlsDefineKey = "GHAL_BOL_COORD_URLS";
  static const _tlsDefineKey = "GHAL_BOL_COORD_INSECURE_TLS";

  static String _trimUrl(String raw) => raw.trim().replaceAll(RegExp(r"/+$"), "");

  /// Splits on comma, semicolon, tab, space, newline, or any mix.
  static final _urlDelimiters = RegExp(r"[\s,;]+");

  /// Test-only entry for [parseUrlsForTest].
  @visibleForTesting
  static List<String> parseUrlsForTest(String raw) => _parseUrls(raw);

  static List<String> _parseUrls(String raw) {
    final t = raw.trim();
    if (t.isEmpty) return [];
    if (t.startsWith("[")) {
      try {
        final list = jsonDecode(t) as List<dynamic>;
        return list
            .map((e) => _trimUrl(e.toString()))
            .where((s) => s.isNotEmpty)
            .toList();
      } catch (_) {}
    }
    return t
        .split(_urlDelimiters)
        .map(_trimUrl)
        .where((s) => s.isNotEmpty)
        .toList();
  }

  static List<String>? _fromBuildConfig() {
    const fromUrlsDefine =
        String.fromEnvironment(_urlsDefineKey, defaultValue: "");
    if (fromUrlsDefine.trim().isNotEmpty) {
      final parsed = _parseUrls(fromUrlsDefine);
      if (parsed.isNotEmpty) return parsed;
    }
    final shellUrls = Platform.environment[_urlsDefineKey]?.trim();
    if (shellUrls != null && shellUrls.isNotEmpty) {
      final parsed = _parseUrls(shellUrls);
      if (parsed.isNotEmpty) return parsed;
    }
    final fileUrls = AppEnvConfig.get(_urlsDefineKey);
    if (fileUrls != null && fileUrls.isNotEmpty) {
      final parsed = _parseUrls(fileUrls);
      if (parsed.isNotEmpty) return parsed;
    }
    return null;
  }

  /// Non-empty when build/env configured at least one coord server.
  static List<String> get defaultBaseUrls {
    final configured = _fromBuildConfig();
    return configured ?? [];
  }

  /// Includes persisted native preference when build/env did not set URLs.
  static Future<List<String>> effectiveBaseUrls() async {
    final configured = defaultBaseUrls;
    if (configured.isNotEmpty) return configured;
    final prefs = GhalBolFfi.coordSettingsGet(appNamespace: kGhalBolAppNamespace);
    final urlsRaw = prefs?["base_urls"];
    if (urlsRaw is List) {
      final urls = urlsRaw
          .map((e) => _trimUrl(e.toString()))
          .where((s) => s.isNotEmpty)
          .toList();
      if (urls.isNotEmpty) return urls;
    }
    return [];
  }

  static Future<bool> effectiveInsecureTls() async {
    const fromDefine = String.fromEnvironment(_tlsDefineKey, defaultValue: "");
    if (fromDefine == "1" || fromDefine.toLowerCase() == "true") return true;
    final shell = Platform.environment[_tlsDefineKey]?.trim() ?? "";
    if (shell == "1" || shell.toLowerCase() == "true") return true;
    final file = AppEnvConfig.get(_tlsDefineKey);
    if (file == "1" || file?.toLowerCase() == "true") return true;
    final prefs = GhalBolFfi.coordSettingsGet(appNamespace: kGhalBolAppNamespace);
    return prefs?["insecure_tls"] == true;
  }

  static bool get isConfigured => defaultBaseUrls.isNotEmpty;

  static bool get defaultInsecureTls {
    const fromEnv = String.fromEnvironment(_tlsDefineKey, defaultValue: "");
    if (fromEnv == "1" || fromEnv.toLowerCase() == "true") return true;
    final shell = Platform.environment[_tlsDefineKey]?.trim() ?? "";
    if (shell == "1" || shell.toLowerCase() == "true") return true;
    final file = AppEnvConfig.get(_tlsDefineKey);
    return file == "1" || file?.toLowerCase() == "true";
  }
}
