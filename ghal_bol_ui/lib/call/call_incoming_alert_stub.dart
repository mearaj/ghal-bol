abstract final class CallIncomingAlert {
  static void installOpenedHandler(void Function() onOpened) {}

  static Future<void> show({
    required String displayName,
    required String publicKeyHex,
  }) async {}

  static Future<void> dismiss() async {}

  static Future<void> presentWindow() async {}
}
