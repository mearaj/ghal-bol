import "dart:io" show Platform;

import "package:ghal_bol_ui/daemon_client_api.dart";
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

  /// Lightweight re-unlock (poll bridge, session refresh) — no socket teardown.
  static Future<Map<String, dynamic>> unlock({
    required String appNamespace,
    required String password,
  }) async {
    if (!isSupported) return {"ok": false, "error": "daemon not supported"};
    SessionFlowLog.daemon("unlock_request", {"ns": appNamespace});
    var r = await GhalBolDaemonClient.instance.unlock(
      appNamespace: appNamespace,
      password: password,
    );
    if (r["ok"] == true) return r;
    final err = r["error"]?.toString();
    if (!_isRecoverableUnlockError(err)) {
      SessionFlowLog.daemonIssue("unlock_failed", detail: err);
      return r;
    }
    SessionFlowLog.daemon("unlock_recover", {"reason": err ?? "unknown"});
    await GhalBolDaemonClient.reconnectDaemon();
    r = await GhalBolDaemonClient.instance.unlock(
      appNamespace: appNamespace,
      password: password,
    );
    SessionFlowLog.daemonIssue("unlock_failed", detail: r["error"]?.toString());
    return r;
  }

  static bool _isRecoverableUnlockError(String? err) {
    if (err == null || err.isEmpty) return false;
    final low = err.toLowerCase();
    return low.contains("disconnected") ||
        low.contains("not running") ||
        low.contains("broken pipe") ||
        low.contains("connection reset") ||
        low.contains("connection refused") ||
        low.contains("timed out");
  }

  /// Fresh sign-in / UI-lock resume — caller must [prepareForLoginUnlock] first when needed.
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
      DaemonMethod.sessionUnlocked,
      ensureDaemon: false,
    );
    return r["ok"] == true && r["unlocked"] == true;
  }

  static Future<void> installLinuxAutostart() =>
      GhalBolDaemonClient.installLinuxAutostart();

  static Future<void> removeLinuxAutostart() =>
      GhalBolDaemonClient.removeLinuxAutostart();

  /// Linux: mark UI process running so daemon grace timer skips unlock wake.
  static Future<void> touchLinuxUiPresence() =>
      GhalBolDaemonClient.touchLinuxUiPresence();

  static Future<void> clearLinuxUiPresence() =>
      GhalBolDaemonClient.clearLinuxUiPresence();

  /// OS network truth from `:p2p` / `ghal_bol_daemon` (`android_network` / `linux_network`).
  static Future<Map<String, dynamic>?> networkSnapshot() async {
    if (!isSupported) return null;
    if (!await GhalBolDaemonClient.probeDaemon()) return null;
    final r = await GhalBolDaemonClient.instance.callState(
      DaemonMethod.networkSnapshot,
      ensureDaemon: false,
    );
    if (r["ok"] != true) return null;
    return r;
  }
}
