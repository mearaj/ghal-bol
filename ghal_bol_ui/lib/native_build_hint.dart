import "dart:io";

/// Platform-correct command to rebuild `lib_ghal_bol_core` for the current target.
abstract final class NativeBuildHint {
  static String get rebuildFromRepoRoot {
    if (Platform.isAndroid) {
      return "./scripts/pack_android_workspace_jni_libs.sh";
    }
    return "./scripts/sync_ghal_bol_native_for_flutter.sh";
  }

  static String get rebuildInstructions {
    if (Platform.isAndroid) {
      return "From the repo root, run $rebuildFromRepoRoot (needs cargo-ndk and "
          "ANDROID_NDK_HOME), then reinstall the app.";
    }
    return "Quit the app, then from the repo root run $rebuildFromRepoRoot and restart.";
  }

  static String get libraryUnavailable =>
      "Could not load lib_ghal_bol_core. $rebuildInstructions";
}
