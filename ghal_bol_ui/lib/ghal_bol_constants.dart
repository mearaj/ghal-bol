import "package:flutter/foundation.dart";

/// Production id — matches [`ghal_bol_core::ANDROID_LIBRARY_NAMESPACE`] and release `applicationId`.
const String kGhalBolProductionNamespace = "com.ghalbol";

/// Debug sideload namespace (`flutter run` on Android and Linux).
const String kGhalBolDebugNamespace = "com.ghalbol.debug";

/// Must stay in sync with [`ghal_bol_core::ANDROID_LIBRARY_NAMESPACE`].
const String kGhalBolAndroidLibraryNamespace = kGhalBolProductionNamespace;

/// FFI/P2P `app_namespace` for this process.
///
/// | Platform | Debug (`flutter run`) | Release |
/// |----------|----------------------|---------|
/// | Android | `com.ghalbol.debug` | `com.ghalbol` |
/// | Linux | `com.ghalbol.debug` | `com.ghalbol` |
String get kGhalBolAppNamespace {
  if (kIsWeb) return kGhalBolProductionNamespace;
  return kDebugMode ? kGhalBolDebugNamespace : kGhalBolProductionNamespace;
}
