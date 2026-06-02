import "dart:io" show Platform;

import "package:ghal_bol_ui/user_flow_log.dart";
import "package:ghal_bol_ui/src/ghal_bol_daemon_client_io.dart"
    if (dart.library.html) "package:ghal_bol_ui/src/ghal_bol_daemon_client_stub.dart";

/// Out-of-process P2P (Linux `ghal_bol_daemon`, Android `:p2p` foreground service).
abstract final class GhalBolDaemon {
  static bool get isSupported => Platform.isLinux || Platform.isAndroid;

  static Future<void> ensureRunning() async {
    if (!isSupported) return;
    await GhalBolDaemonClient.ensureDaemonRunning();
  }

  /// Call before unlock UI submits — clears dead sockets and restarts `:p2p` if needed.
  static Future<void> prepareForLoginUnlock() async {
    if (!isSupported) return;
    await GhalBolDaemonClient.prepareForLoginUnlock();
  }

  static Future<Map<String, dynamic>> unlock({
    required String appNamespace,
    required String password,
  }) async {
    if (!isSupported) return {"ok": false, "error": "daemon not supported"};
    return unlockWithRecovery(appNamespace: appNamespace, password: password);
  }

  /// Unlock in P2P process; retries after disconnect / restarts `:p2p` on Android.
  static Future<Map<String, dynamic>> unlockWithRecovery({
    required String appNamespace,
    required String password,
  }) async {
    if (!isSupported) return {"ok": false, "error": "daemon not supported"};
    SessionFlowLog.daemon("unlock_request", {"ns": appNamespace});
    final r = await GhalBolDaemonClient.instance.unlockWithRecovery(
      appNamespace: appNamespace,
      password: password,
    );
    if (r["ok"] != true) {
      SessionFlowLog.daemonIssue(
        "unlock_failed",
        detail: r["error"]?.toString(),
      );
    }
    return r;
  }

  static Future<void> stopSession() async {
    if (!isSupported) return;
    await GhalBolDaemonClient.instance.stopSession();
  }

  static Future<bool> sessionUnlocked() async {
    if (!isSupported) return false;
    if (!await GhalBolDaemonClient.probeDaemon()) return false;
    final r = await GhalBolDaemonClient.instance.call(
      "session_unlocked",
      ensureDaemon: false,
    );
    return r["ok"] == true && r["unlocked"] == true;
  }
}
