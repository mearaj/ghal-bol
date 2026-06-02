import "dart:io";

import "package:ghal_bol_ui/app_env_config.dart";
import "package:ghal_bol_ui/app_log.dart";
import "package:ghal_bol_ui/embedder_storage.dart";
import "package:ghal_bol_ui/ghal_bol_ffi.dart";

/// Host bootstrap before [runApp]: env, native library, embedder storage alignment.
Future<void> ghalBolHostInitBeforeRunApp() async {
  await AppEnvConfig.load();
  GhalBolFfi.tryInitLibrary();
  await ghalBolAlignNativeStorage();
  if (!GhalBolFfi.isLibraryLoaded) {
    AppLog.instance.w("Host", "native library not loaded: ${GhalBolFfi.loadErrorText}");
  } else {
    AppLog.instance.i(
      "Host",
      "native loaded p2p=${GhalBolFfi.isP2pAvailable} coord=${GhalBolFfi.isCoordAvailable} "
      "contacts=${GhalBolFfi.isContactsStoreAvailable} "
      "daemon=${Platform.isLinux || Platform.isAndroid}",
    );
  }
}
