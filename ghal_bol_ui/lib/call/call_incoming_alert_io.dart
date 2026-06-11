import "package:flutter/foundation.dart";
import "package:flutter/services.dart";

import "package:ghal_bol_ui/ghal_bol_p2p.dart";

abstract final class CallIncomingAlert {
  static const MethodChannel _channel = MethodChannel("ghal_bol/incoming_call");
  static bool _handlerInstalled = false;

  /// Wire platform → Dart callbacks (notification tap, GTK close X).
  static void installPlatformHandlers({
    void Function({String? publicKeyHex, String? displayName})? onOpenedFromNotification,
    void Function()? onWindowClosedByUser,
  }) {
    if (_handlerInstalled) return;
    _handlerInstalled = true;
    _channel.setMethodCallHandler((call) async {
      switch (call.method) {
        case "openedFromNotification":
          final args = call.arguments;
          if (args is Map) {
            onOpenedFromNotification?.call(
              publicKeyHex: args["publicKeyHex"]?.toString(),
              displayName: args["displayName"]?.toString(),
            );
          } else {
            onOpenedFromNotification?.call();
          }
        case "windowClosedByUser":
          onWindowClosedByUser?.call();
      }
    });
  }

  static void installOpenedHandler(
    void Function({String? publicKeyHex, String? displayName}) onOpened,
  ) {
    installPlatformHandlers(onOpenedFromNotification: onOpened);
  }

  /// OS notification (Android full-screen / Linux libnotify). Does not raise the window on Linux.
  static Future<void> show({
    required String displayName,
    required String publicKeyHex,
  }) async {
    try {
      await _channel.invokeMethod<void>("show", {
        "displayName": displayName,
        "publicKeyHex": publicKeyHex,
      });
    } catch (_) {}
  }

  static Future<void> dismiss() async {
    try {
      if (!kIsWeb &&
          (defaultTargetPlatform == TargetPlatform.linux ||
              defaultTargetPlatform == TargetPlatform.android)) {
        await GhalBolP2p.dismissIncomingCallAlert();
      }
      await _channel.invokeMethod<void>("dismiss");
    } catch (_) {}
  }

  /// Whether the main window is visible (Linux desktop).
  static Future<bool> isWindowVisible() async {
    if (defaultTargetPlatform != TargetPlatform.linux) return true;
    try {
      final v = await _channel.invokeMethod<bool>("isWindowVisible");
      return v ?? true;
    } catch (_) {
      return true;
    }
  }

  /// Raise the main window without posting a new notification.
  static Future<void> presentWindow() async {
    try {
      await _channel.invokeMethod<void>("present");
    } catch (_) {}
  }

  /// Hide main window after active calls are torn down (GTK close X).
  static Future<void> hideWindow() async {
    if (defaultTargetPlatform != TargetPlatform.linux) return;
    try {
      await _channel.invokeMethod<void>("hideWindow");
    } catch (_) {}
  }

  /// Exit the GTK app (Linux only) so `flutter run` terminates when idle.
  static Future<void> quitApplication() async {
    if (defaultTargetPlatform != TargetPlatform.linux) return;
    try {
      await _channel.invokeMethod<void>("quitApplication");
    } catch (_) {}
  }
}
