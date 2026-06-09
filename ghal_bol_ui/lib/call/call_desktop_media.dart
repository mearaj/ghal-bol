import "package:flutter/foundation.dart";

/// Desktop platform helper for calls. Native-only — no WebRTC.
abstract final class CallDesktopMedia {
  static bool get isDesktopNative =>
      !kIsWeb &&
      (defaultTargetPlatform == TargetPlatform.linux ||
          defaultTargetPlatform == TargetPlatform.windows ||
          defaultTargetPlatform == TargetPlatform.macOS);
}
