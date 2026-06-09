import "dart:async";

import "package:ghal_bol_ui/call/call_controller.dart";
import "package:ghal_bol_ui/user_flow_log.dart";
import "package:ghal_bol_ui/ghal_bol_coord.dart";
import "package:ghal_bol_ui/ghal_bol_daemon.dart";
import "package:ghal_bol_ui/ghal_bol_listener_foreground.dart";
import "package:ghal_bol_ui/ghal_bol_p2p.dart";
import "package:ghal_bol_ui/p2p_event_bridge.dart";
import "package:ghal_bol_ui/p2p_network_coordinator.dart";
import "package:ghal_bol_ui/session_credentials.dart";
import "package:ghal_bol_ui/src/ghal_bol_ffi_result.dart";

/// Keeps native libp2p + the poll loop alive when the chat UI is torn down (lock, hub dispose).
///
/// Stop only via [stopForLogout] (logout, delete identity, or full sign-out flows).
class GhalBolBackground {
  GhalBolBackground._();

  /// Start poll loop + P2P bootstrap after unlock (does not block the unlock UI).
  static Future<void> ensureRunning(GhalBolIdentityResult session) async {
    SessionFlowLog.step("background_start", {
      "ns": session.appNamespace ?? "(default)",
      "pk": SessionFlowLog.shortPk(session.publicKeyHex),
    });
    P2pNetworkCoordinator.markSessionRefresh();
    await GhalBolCoord.configureAfterUnlock();
    await P2pEventBridge.instance.ensureStarted(session);
  }

  /// Foreground service + drain when the process resumes (hub may be absent after UI lock).
  static Future<void> onAppResumed() async {
    if (!SessionCredentials.hasPassword) return;
    CallController.instance.onAppForeground();
    if (await GhalBolP2p.isRunning()) {
      unawaited(ghalBolListenerForegroundEnsureStarted());
      unawaited(GhalBolP2p.notifyNetworkChange());
      P2pEventBridge.instance.drainNow();
    } else {
      unawaited(P2pEventBridge.instance.recoverP2pIfNeeded());
    }
  }

  /// Logout / delete identity: tear down listener, poll loop, and native P2P.
  static Future<void> stopForLogout() async {
    SessionFlowLog.step("logout_stop");
    SessionCredentials.clear();
    await P2pEventBridge.instance.stop();
    P2pNetworkCoordinator.invalidate();
    await ghalBolListenerForegroundStop();
    if (GhalBolDaemon.isSupported) {
      await GhalBolDaemon.stopSession();
    } else {
      await GhalBolP2p.stop();
    }
  }
}
