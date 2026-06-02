import "dart:io" show Platform;

import "package:ghal_bol_ui/app_env_config.dart";
import "package:ghal_bol_ui/ghal_bol_ffi.dart";
import "package:ghal_bol_ui/ghal_bol_constants.dart";

/// Coordination server base URL (presence + endpoint lookup).
///
/// Resolved from `--dart-define`, OS env, bundled `env/.env.*` ([AppEnvConfig]),
/// platform defaults, then native preferences.
abstract final class CoordinationUrl {
  static const _defineKey = "GHAL_BOL_COORD_URL";
  static const _tlsDefineKey = "GHAL_BOL_COORD_INSECURE_TLS";
  static const _emulatorFlagKey = "GHAL_BOL_ANDROID_EMULATOR";

  static bool get _androidEmulatorBuild {
    const v = String.fromEnvironment(_emulatorFlagKey, defaultValue: "");
    return v == "1" || v.toLowerCase() == "true";
  }

  static String _trimUrl(String raw) => raw.trim().replaceAll(RegExp(r"/+$"), "");

  static String? _fromBuildConfig() {
    const fromDefine = String.fromEnvironment(_defineKey, defaultValue: "");
    if (fromDefine.trim().isNotEmpty) return _trimUrl(fromDefine);
    final shell = Platform.environment[_defineKey]?.trim();
    if (shell != null && shell.isNotEmpty) return _trimUrl(shell);
    final file = AppEnvConfig.get(_defineKey);
    if (file != null && file.isNotEmpty) return _trimUrl(file);
    return null;
  }

  /// Non-empty when a coord server should be used for this build/session.
  static String get defaultBaseUrl {
    final configured = _fromBuildConfig();
    if (configured != null && configured.isNotEmpty) return configured;
    if (Platform.isAndroid && _androidEmulatorBuild) {
      return "http://10.0.2.2:8765";
    }
    if (Platform.isLinux || Platform.isMacOS || Platform.isWindows) {
      return "http://127.0.0.1:8765";
    }
    return "";
  }

  /// Includes persisted native preference when build/env did not set a URL.
  static Future<String> effectiveBaseUrl() async {
    final configured = defaultBaseUrl;
    if (configured.isNotEmpty) return configured;
    final prefs = GhalBolFfi.coordSettingsGet(appNamespace: kGhalBolAndroidLibraryNamespace);
    final url = prefs?["base_url"]?.toString().trim() ?? "";
    if (url.isEmpty) return "";
    return _trimUrl(url);
  }

  static Future<bool> effectiveInsecureTls() async {
    const fromDefine = String.fromEnvironment(_tlsDefineKey, defaultValue: "");
    if (fromDefine == "1" || fromDefine.toLowerCase() == "true") return true;
    final shell = Platform.environment[_tlsDefineKey]?.trim() ?? "";
    if (shell == "1" || shell.toLowerCase() == "true") return true;
    final file = AppEnvConfig.get(_tlsDefineKey);
    if (file == "1" || file?.toLowerCase() == "true") return true;
    final prefs = GhalBolFfi.coordSettingsGet(appNamespace: kGhalBolAndroidLibraryNamespace);
    return prefs?["insecure_tls"] == true;
  }

  static bool get isConfigured => defaultBaseUrl.isNotEmpty;

  static bool get defaultInsecureTls {
    const fromEnv = String.fromEnvironment(_tlsDefineKey, defaultValue: "");
    if (fromEnv == "1" || fromEnv.toLowerCase() == "true") return true;
    final shell = Platform.environment[_tlsDefineKey]?.trim() ?? "";
    if (shell == "1" || shell.toLowerCase() == "true") return true;
    final file = AppEnvConfig.get(_tlsDefineKey);
    return file == "1" || file?.toLowerCase() == "true";
  }
}
