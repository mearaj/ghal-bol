abstract final class CallIncomingAlert {
  static void installPlatformHandlers({
    void Function()? onOpenedFromNotification,
    void Function()? onWindowClosedByUser,
  }) {}

  static void installOpenedHandler(void Function() onOpened) {
    installPlatformHandlers(onOpenedFromNotification: onOpened);
  }

  static Future<void> show({
    required String displayName,
    required String publicKeyHex,
  }) async {}

  static Future<void> dismiss() async {}

  static Future<bool> isWindowVisible() async => true;

  static Future<void> presentWindow() async {}

  static Future<void> hideWindow() async {}

  static Future<void> quitApplication() async {}
}
