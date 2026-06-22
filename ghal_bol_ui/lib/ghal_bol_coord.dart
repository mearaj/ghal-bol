import "dart:io";

import "package:ghal_bol_ui/app_log.dart";
import "package:ghal_bol_ui/coordination_url.dart";
import "package:ghal_bol_ui/native_build_hint.dart";
import "package:ghal_bol_ui/ghal_bol_ffi.dart";
import "package:ghal_bol_ui/ghal_bol_p2p.dart";
import "package:ghal_bol_ui/src/ghal_bol_daemon_client_io.dart"
    if (dart.library.html) "package:ghal_bol_ui/src/ghal_bol_daemon_client_stub.dart";

/// Tier-1 coordination server (presence + endpoint lookup).
abstract final class GhalBolCoord {
  static bool get usesDaemon => GhalBolP2p.usesDaemon;

  static bool get hasNativeCoord => GhalBolFfi.isCoordAvailable;

  static bool get isLookupEnabled => usesDaemon || hasNativeCoord;

  static Future<Map<String, dynamic>> setBaseUrls({
    List<String>? baseUrls,
    bool? insecureTls,
  }) async {
    final resolved = baseUrls ?? await CoordinationUrl.effectiveBaseUrls();
    final cleaned =
        resolved.map((u) => u.trim()).where((u) => u.isNotEmpty).toList();
    final tls = insecureTls ?? await CoordinationUrl.effectiveInsecureTls();
    AppLog.instance.i(
      "Coord",
      "set_base_url urls=$cleaned insecure_tls=$tls",
    );
    if (cleaned.isEmpty) {
      return {"ok": false, "error": "coord URL not configured"};
    }
    if (usesDaemon) {
      await GhalBolDaemonClient.ensureDaemonRunning();
      return GhalBolDaemonClient.instance.call(
        "coord_set_base_url",
        params: {
          "base_urls": cleaned,
          "insecure_tls": tls,
        },
      );
    }
    return GhalBolFfi.coordSetBaseUrls(
      baseUrls: cleaned,
      insecureTls: tls,
    );
  }

  // Coord lookup and presence register run entirely in `ghal_bol` (`:p2p` / daemon coord tick) —
  // single source of truth. Flutter must not HTTP-lookup or run register retry loops; it only
  // pushes the coord URL at unlock (`configureAfterUnlock`) and health-checks (`_probeHealth`).

  static Future<Map<String, dynamic>> p2pConfigFields() async {
    final urls = await CoordinationUrl.effectiveBaseUrls();
    return {
      if (urls.isNotEmpty) "coord_base_urls": urls,
      "coord_insecure_tls": await CoordinationUrl.effectiveInsecureTls(),
    };
  }

  /// Push coord URL into `:p2p` / daemon before [GhalBolP2p.startJson].
  static Future<void> configureAfterUnlock() async {
    if (!isLookupEnabled) {
      AppLog.instance.w(
        "Coord",
        "coord unavailable — ${NativeBuildHint.rebuildInstructions}",
      );
      return;
    }
    final urls = await CoordinationUrl.effectiveBaseUrls();
    if (urls.isEmpty) {
      AppLog.instance.i(
        "Coord",
        "coord URL not set — add env/.env.development, --dart-define, or export GHAL_BOL_COORD_URLS",
      );
      return;
    }
    final r = await setBaseUrls(baseUrls: urls);
    if (r["ok"] != true) {
      AppLog.instance.w("Coord", "set_base_url failed: ${r["error"]}");
    }
  }

  /// After [GhalBolP2p] is up: health check only. Presence register runs in `:p2p`
  /// (listen snapshot + coord tick) — do not block the daemon main socket with HTTP.
  static Future<void> registerAndVerifyAfterP2pUp() async {
    final urls = await CoordinationUrl.effectiveBaseUrls();
    if (urls.isEmpty) return;
    for (final url in urls) {
      await _probeHealth(url);
    }
  }

  static Future<void> _probeHealth(String baseUrl) async {
    try {
      final uri = Uri.parse("$baseUrl/health");
      final client = HttpClient();
      client.connectionTimeout = const Duration(seconds: 3);
      final req = await client.getUrl(uri).timeout(const Duration(seconds: 4));
      final resp = await req.close().timeout(const Duration(seconds: 4));
      if (resp.statusCode == 200) {
        AppLog.instance.i("Coord", "health ok $baseUrl");
        return;
      }
      AppLog.instance.w("Coord", "health $baseUrl → HTTP ${resp.statusCode}");
    } catch (e) {
      AppLog.instance.w(
        "Coord",
        "cannot reach coord at $baseUrl ($e) — wrong host, firewall, or server down",
      );
    }
  }
}
