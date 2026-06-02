/// Wasm / non-Linux: no out-of-process daemon.
class GhalBolDaemonClient {
  GhalBolDaemonClient._();
  static final GhalBolDaemonClient instance = GhalBolDaemonClient._();

  static Future<void> ensureDaemonRunning() async {}

  static Future<void> prepareForLoginUnlock() async {}

  static Future<void> forceRecoverDaemon() async {}

  static Future<bool> probeDaemon({bool force = false}) async => false;

  static void invalidateProbeCache() {}

  Future<Map<String, dynamic>> call(
    String method, {
    Map<String, dynamic> params = const {},
    bool ensureDaemon = true,
  }) async =>
      {"ok": false, "error": "daemon not supported"};

  Future<Map<String, dynamic>> unlock({
    required String appNamespace,
    required String password,
  }) async =>
      {"ok": false, "error": "daemon not supported"};

  Future<Map<String, dynamic>> unlockWithRecovery({
    required String appNamespace,
    required String password,
  }) async =>
      {"ok": false, "error": "daemon not supported"};

  Future<void> disconnect() async {}

  Future<void> stopSession() async {}
}
