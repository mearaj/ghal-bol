import "package:ghal_bol_ui/ghal_bol_daemon.dart";
import "package:ghal_bol_ui/p2p_network_coordinator.dart";
import "package:ghal_bol_ui/user_flow_log.dart";

/// App password held in RAM for the unlocked session (cleared on [clear] / logout).
///
/// Used to re-unlock the out-of-process P2P daemon after `ghal_bol_core_daemon` restarts
/// while the UI process still has FFI identity loaded.
class SessionCredentials {
  SessionCredentials._();

  static String? _appNamespace;
  static String? _password;

  static void store({
    required String appNamespace,
    required String password,
  }) {
    _appNamespace = appNamespace.trim();
    _password = password;
  }

  static void clear() {
    _appNamespace = null;
    _password = null;
  }

  static bool get hasPassword =>
      (_password?.isNotEmpty ?? false) && (_appNamespace?.isNotEmpty ?? false);

  /// True when daemon session is unlocked, or after a successful re-unlock.
  static Future<bool> ensureDaemonUnlocked() async {
    if (!GhalBolDaemon.isSupported) return true;
    await GhalBolDaemon.ensureRunning();
    if (await GhalBolDaemon.sessionUnlocked()) return true;
    final ns = _appNamespace;
    final pw = _password;
    if (ns == null || pw == null || pw.isEmpty) {
      SessionFlowLog.daemonIssue(
        "daemon_not_unlocked",
        check: "lock and unlock the app to restore chat/P2P",
      );
      return false;
    }
    SessionFlowLog.daemon("re_unlock_start", {"ns": ns});
    final dr = await GhalBolDaemon.unlock(appNamespace: ns, password: pw);
    if (dr["ok"] != true) {
      SessionFlowLog.daemonIssue(
        "re_unlock_failed",
        detail: dr["error"]?.toString(),
        check: "grep Daemon prepare_login / forceRecover",
      );
      return false;
    }
    P2pNetworkCoordinator.markSessionRefresh();
    SessionFlowLog.daemon("re_unlock_ok");
    return true;
  }
}
