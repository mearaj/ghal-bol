import "dart:io";

import "package:ghal_bol_ui/ghal_bol_daemon.dart";
import "package:ghal_bol_ui/ghal_bol_ffi.dart";

/// Thin platform glue — all probe logic lives in `ghal_bol` (`:p2p` / daemon / FFI).
abstract final class NetworkHelperPlatform {
  static String get platformLabel {
    if (GhalBolDaemon.isSupported) return "daemon_rpc";
    if (Platform.isLinux) return "linux_ffi";
    return Platform.operatingSystem;
  }

  static Future<Map<String, dynamic>?> fetchSnapshot() async {
    if (GhalBolDaemon.isSupported) {
      return GhalBolDaemon.networkSnapshot();
    }
    if (Platform.isLinux && GhalBolFfi.isNetworkHelperAvailable) {
      return GhalBolFfi.networkSnapshot();
    }
    return null;
  }
}
