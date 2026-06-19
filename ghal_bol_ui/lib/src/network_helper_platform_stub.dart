/// Non-IO stub — [NetworkHelper] stays unknown on web.
abstract final class NetworkHelperPlatform {
  static String get platformLabel => "stub";

  static Future<Map<String, dynamic>?> fetchSnapshot() async => null;
}
