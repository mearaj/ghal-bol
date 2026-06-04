import "package:flutter/services.dart";

abstract final class CallIncomingAlert {
  static const MethodChannel _channel = MethodChannel("ghal_bol/incoming_call");
  static bool _handlerInstalled = false;

  /// Wire platform → Dart when user taps the full-screen incoming-call notification.
  static void installOpenedHandler(void Function() onOpened) {
    if (_handlerInstalled) return;
    _handlerInstalled = true;
    _channel.setMethodCallHandler((call) async {
      if (call.method == "openedFromNotification") {
        onOpened();
      }
    });
  }

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
      await _channel.invokeMethod<void>("dismiss");
    } catch (_) {}
  }

  /// Linux desktop: raise the window to the foreground.
  static Future<void> presentWindow() async {
    try {
      await _channel.invokeMethod<void>("present");
    } catch (_) {}
  }
}
