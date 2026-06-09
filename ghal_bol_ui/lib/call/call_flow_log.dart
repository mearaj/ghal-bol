import "package:ghal_bol_ui/app_log.dart";

/// Voice/video call lines in App log ([logCallFlow] only — not [logUserFlow]).
abstract final class CallFlowLog {
  static String? _callId;

  static void bindCall(String? callId) {
    _callId = callId?.trim().isEmpty == true ? null : callId?.trim();
  }

  static void step(String step, [Map<String, String>? fields]) {
    AppLog.instance.callStep("Call", step, _withCall(fields));
  }

  static void media(String step, [Map<String, String>? fields]) {
    AppLog.instance.callStep("Call/Media", step, _withCall(fields));
  }

  static void mediaDetail(String step, String detail) {
    AppLog.instance.callDetail("Call/Media", step, _detail(detail));
  }

  static void issue(
    String problem, {
    String? check,
    String? detail,
  }) {
    AppLog.instance.callIssue(
      "Call",
      problem,
      check: check,
      detail: detail,
      callId: _callId,
    );
  }

  static String _detail(String detail) {
    final id = _callId;
    if (id == null || id.isEmpty) return detail;
    return "call_id=$id | $detail";
  }

  static Map<String, String>? _withCall(Map<String, String>? fields) {
    if (_callId == null && fields == null) return null;
    return {
      "call_id": _callId ?? "?",
      ...?fields,
    };
  }

  static String shortPk(String? hex) {
    final s = hex?.trim() ?? "";
    if (s.length <= 10) return s.isEmpty ? "?" : s;
    return "${s.substring(0, 8)}…";
  }
}
