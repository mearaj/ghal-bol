import "package:ghal_bol_ui/app_log.dart";

/// Unlock / login / daemon session (grep `Session` or `Daemon` + `step=`).
abstract final class SessionFlowLog {
  static void step(String step, [Map<String, String>? fields]) {
    AppLog.instance.journey("Session", step, fields);
  }

  static void daemon(String step, [Map<String, String>? fields]) {
    AppLog.instance.journey("Daemon", step, fields);
  }

  static void issue(
    String problem, {
    String? check,
    String? detail,
  }) {
    AppLog.instance.hint("Session", problem, check: check, detail: detail);
  }

  static void daemonIssue(
    String problem, {
    String? check,
    String? detail,
  }) {
    AppLog.instance.hint("Daemon", problem, check: check, detail: detail);
  }

  static String shortPk(String? hex) {
    final s = hex?.trim() ?? "";
    if (s.length <= 10) return s.isEmpty ? "?" : s;
    return "${s.substring(0, 8)}…";
  }
}

/// P2P connect / dial / stream_ready chain (grep `P2P` + `step=` or `issue=`).
abstract final class P2pFlowLog {
  static void step(String step, [Map<String, String>? fields]) {
    AppLog.instance.journey("P2P", step, fields);
  }

  static void coord(String step, [Map<String, String>? fields]) {
    AppLog.instance.journey("P2P/Coord", step, fields);
  }

  static void detail(String step, String detail) {
    AppLog.instance.journeyDetail("P2P", step, detail, null);
  }

  static void issue(
    String problem, {
    String? check,
    String? detail,
  }) {
    AppLog.instance.hint("P2P", problem, check: check, detail: detail);
  }

  static String shortPk(String? hex) {
    final s = hex?.trim() ?? "";
    if (s.length <= 10) return s.isEmpty ? "?" : s;
    return "${s.substring(0, 8)}…";
  }

  static String shortPeer(Object? peerOrPk) {
    final s = peerOrPk?.toString().trim() ?? "";
    if (s.isEmpty) return "?";
    if (s.length == 64) return shortPk(s);
    if (s.length > 12) return "${s.substring(0, 8)}…";
    return s;
  }
}
