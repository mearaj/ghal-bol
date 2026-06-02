import "dart:convert";

import "package:flutter/foundation.dart";

/// In-app diagnostic log (session buffer). Never logs unlock passwords or private keys.
///
/// Structured user-journey lines (grep in export):
/// - `#N step=…` — numbered user journey ([logUserFlow] + per-domain toggles)
/// - `[Call]` / `[Session]` / `[Daemon]` / `[P2P]` — domain tags
/// - `issue=… | check=…` — warning + what to verify
/// - `[Trace] #N` — DM delivery chain ([logMessageFlow])
class AppLog extends ChangeNotifier {
  AppLog._();

  static final AppLog instance = AppLog._();

  static const int maxEntries = 2000;

  /// Android Binder/clipboard limit — keep copy/share payloads under this.
  static const int maxExportBytes = 256 * 1024;

  /// When false, [logP2pEvent] skips `native_log` debug lines (default: off for performance).
  static bool logNativeDebug = false;

  /// When true, logs DM send/receive/store flow in UI (Contacts, Transcript, Chat, Retry).
  static bool logMessageFlow = true;

  /// Master switch for structured journey lines (`step=`, `issue=`).
  static bool logUserFlow = true;

  /// When true, logs voice/video call steps.
  static bool logCallFlow = true;

  /// When true, logs unlock / login / daemon session steps.
  static bool logSessionFlow = true;

  /// When true, logs P2P connect / dial / stream_ready steps.
  static bool logP2pFlow = true;

  /// Wall-clock start of this in-memory log buffer (reset on [clear]).
  DateTime sessionStartedAt = DateTime.now();

  int _traceSeq = 0;
  int _journeySeq = 0;
  int _callStepSeq = 0;

  final List<AppLogEntry> entries = [];
  DateTime? _lastNotifyAt;

  static final RegExp _jsonSensitiveStringField = RegExp(
    r'"(password|passphrase|unlock_password|secret|private_key|private_key_hex|seed_phrase|mnemonic)"\s*:\s*"[^"]*"',
    caseSensitive: false,
  );

  static final RegExp _queryPassword = RegExp(
    r"(password|passphrase)=[^&\s]+",
    caseSensitive: false,
  );

  static const Set<String> _redactJsonKeys = {
    "password",
    "passphrase",
    "unlock_password",
    "secret",
    "private_key",
    "private_key_hex",
    "seed",
    "seed_phrase",
    "mnemonic",
    "signing_private_key_hex",
    "encryption_private_key_hex",
  };

  void d(String tag, String message) => _add(AppLogLevel.debug, tag, message);

  void i(String tag, String message) => _add(AppLogLevel.info, tag, message);

  void w(String tag, String message) => _add(AppLogLevel.warn, tag, message);

  void e(String tag, String message, [Object? err, StackTrace? st]) {
    var msg = message;
    if (err != null) msg = "$msg — $err";
    if (st != null) msg = "$msg\n$st";
    _add(AppLogLevel.error, tag, msg);
  }

  /// Log a map/list after redacting sensitive keys (passwords, private keys).
  void json(String tag, String action, Object? value) {
    final body = value == null ? "" : " ${formatPayload(value)}";
    i(tag, "$action$body");
  }

  /// End-to-end message path (gated by [logMessageFlow]).
  void flow(String tag, String message) {
    if (!logMessageFlow) return;
    i(tag, message);
  }

  void flowJson(String tag, String action, Object? value) {
    if (!logMessageFlow) return;
    json(tag, action, value);
  }

  /// Numbered step along the DESIGN.md delivery chain (grep `Trace` in export).
  void trace(String step, String detail) {
    if (!logMessageFlow) return;
    _traceSeq++;
    i("Trace", "#$_traceSeq $step — $detail");
  }

  bool _journeyEnabled(String tag) {
    if (tag.startsWith("Call")) return logCallFlow;
    if (!logUserFlow) return false;
    if ((tag == "Session" || tag == "Daemon") && !logSessionFlow) return false;
    if (tag.startsWith("P2P") && !logP2pFlow) return false;
    return true;
  }

  bool _hintEnabled(String tag) {
    if (tag.startsWith("Call")) return logCallFlow;
    if (!logUserFlow) return false;
    if ((tag == "Session" || tag == "Daemon") && !logSessionFlow) return false;
    if (tag.startsWith("P2P") && !logP2pFlow) return false;
    return true;
  }

  /// Voice/video call steps — only [logCallFlow]; not tied to [logUserFlow].
  void callStep(String tag, String step, [Map<String, String>? fields]) {
    if (!logCallFlow) return;
    _callStepSeq++;
    final parts = <String>["#$_callStepSeq", "step=$step"];
    if (fields != null) {
      for (final e in fields.entries) {
        final v = e.value.trim();
        if (v.isNotEmpty) parts.add("${e.key}=$v");
      }
    }
    i(tag, parts.join(" | "));
  }

  void callDetail(String tag, String step, String detail) {
    if (!logCallFlow) return;
    i(tag, "step=$step | $detail");
  }

  void callIssue(
    String tag,
    String problem, {
    String? check,
    String? detail,
    String? callId,
  }) {
    if (!logCallFlow) return;
    final parts = <String>["issue=$problem"];
    if (callId != null && callId.isNotEmpty) parts.add("call_id=$callId");
    if (detail != null && detail.trim().isNotEmpty) parts.add(detail.trim());
    if (check != null && check.trim().isNotEmpty) {
      parts.add("check=${check.trim()}");
    }
    w(tag, parts.join(" | "));
  }

  /// Numbered user-journey step (`step=` prefix — grep `step=` or tag in export).
  void journey(String tag, String step, [Map<String, String>? fields]) {
    if (!_journeyEnabled(tag)) return;
    _journeySeq++;
    final parts = <String>["#$_journeySeq", "step=$step"];
    if (fields != null) {
      for (final e in fields.entries) {
        final v = e.value.trim();
        if (v.isNotEmpty) parts.add("${e.key}=$v");
      }
    }
    i(tag, parts.join(" | "));
  }

  /// Extra detail for a journey step (debug level; gated by [logCallFlow]).
  void journeyDetail(
    String tag,
    String step,
    String detail,
    String? callId,
  ) {
    if (!_journeyEnabled(tag)) return;
    final prefix = callId != null && callId.isNotEmpty
        ? "call_id=$callId | "
        : "";
    d(tag, "${prefix}step=$step | $detail");
  }

  /// Warning with optional remediation (`issue=` / `check=` — easy to spot in export).
  void hint(
    String tag,
    String problem, {
    String? check,
    String? detail,
    String? callId,
  }) {
    if (!_hintEnabled(tag)) return;
    final parts = <String>["issue=$problem"];
    if (callId != null && callId.isNotEmpty) parts.add("call_id=$callId");
    if (detail != null && detail.trim().isNotEmpty) {
      parts.add(detail.trim());
    }
    if (check != null && check.trim().isNotEmpty) {
      parts.add("check=${check.trim()}");
    }
    w(tag, parts.join(" | "));
  }

  /// Daemon / P2P JSON-RPC result (never logs passwords or unlock params).
  void rpc(
    String tag,
    String method, {
    required bool ok,
    String? error,
    int? elapsedMs,
    bool stateSocket = false,
  }) {
    final sock = stateSocket ? "state" : "main";
    final parts = <String>["$method socket=$sock"];
    if (elapsedMs != null) parts.add("${elapsedMs}ms");
    if (!ok) {
      parts.add("FAIL");
      if (error != null && error.trim().isNotEmpty) parts.add(error.trim());
      w(tag, parts.join(" "));
      return;
    }
    final important = _importantRpcMethods.contains(method);
    if (important || logMessageFlow) {
      i(tag, "${parts.join(" ")} ok");
    } else if (logNativeDebug) {
      d(tag, "${parts.join(" ")} ok");
    }
  }

  static const Set<String> _importantRpcMethods = {
    "unlock",
    "p2p_start",
    "p2p_stop",
    "p2p_poll",
    "p2p_send_text_dm",
    "p2p_set_foreground_peer",
    "p2p_set_app_ack_read_enabled",
    "p2p_register_dm_peer",
    "p2p_call_signal",
  };

  String sessionUptimeLabel() {
    final d = DateTime.now().difference(sessionStartedAt);
    if (d.inDays > 0) return "${d.inDays}d ${d.inHours % 24}h";
    if (d.inHours > 0) return "${d.inHours}h ${d.inMinutes % 60}m";
    if (d.inMinutes > 0) return "${d.inMinutes}m ${d.inSeconds % 60}s";
    return "${d.inSeconds}s";
  }

  static String formatPayload(Object value) {
    try {
      if (value is Map) {
        return jsonEncode(sanitizeMap(value));
      }
      if (value is List) {
        return jsonEncode(sanitizeValue(value));
      }
      return sanitize(value.toString());
    } catch (_) {
      return sanitize(value.toString());
    }
  }

  static String sanitize(String input) {
    var s = input;
    s = s.replaceAllMapped(
      _jsonSensitiveStringField,
      (m) => '"${m.group(1)}":"[redacted]"',
    );
    s = s.replaceAllMapped(_queryPassword, (m) => "${m.group(1)}=[redacted]");
    return s;
  }

  static dynamic sanitizeValue(dynamic v) {
    if (v is Map) return sanitizeMap(v);
    if (v is List) return v.map(sanitizeValue).toList();
    if (v is String) return sanitize(v);
    return v;
  }

  static Map<String, dynamic> sanitizeMap(Map map) {
    final out = <String, dynamic>{};
    for (final e in map.entries) {
      final key = e.key.toString();
      if (_redactJsonKeys.contains(key.toLowerCase())) {
        out[key] = "[redacted]";
        continue;
      }
      final v = e.value;
      if (v is Map) {
        out[key] = sanitizeMap(v);
      } else if (v is List) {
        out[key] = v.map(sanitizeValue).toList();
      } else if (v is String) {
        out[key] = sanitize(v);
      } else {
        out[key] = v;
      }
    }
    return out;
  }

  void _add(AppLogLevel level, String tag, String message) {
    final line = AppLogEntry(
      at: DateTime.now(),
      level: level,
      tag: tag.trim().isEmpty ? "App" : tag.trim(),
      message: sanitize(message),
    );
    entries.add(line);
    if (entries.length > maxEntries) {
      entries.removeRange(0, entries.length - maxEntries);
    }
    if (kDebugMode && (level != AppLogLevel.debug || logNativeDebug)) {
      // ignore: avoid_print
      print(line.formatLine());
    }
    _notifyThrottled();
  }

  void _notifyThrottled() {
    if (!hasListeners) return;
    final now = DateTime.now();
    final last = _lastNotifyAt;
    if (last != null && now.difference(last).inMilliseconds < 500) return;
    _lastNotifyAt = now;
    notifyListeners();
  }

  void clear() {
    entries.clear();
    sessionStartedAt = DateTime.now();
    _traceSeq = 0;
    _journeySeq = 0;
    _callStepSeq = 0;
    notifyListeners();
  }

  List<AppLogEntry> filtered({
    AppLogLevel? minLevel,
    String? tagContains,
    String? search,
  }) {
    return entries.where((e) {
      if (minLevel != null && e.level.index < minLevel.index) return false;
      if (tagContains != null &&
          tagContains.isNotEmpty &&
          !e.tag.toLowerCase().contains(tagContains.toLowerCase())) {
        return false;
      }
      if (search != null && search.isNotEmpty) {
        final q = search.toLowerCase();
        if (!e.message.toLowerCase().contains(q) &&
            !e.tag.toLowerCase().contains(q) &&
            !e.level.label.toLowerCase().contains(q)) {
          return false;
        }
      }
      return true;
    }).toList();
  }

  String exportText({Iterable<AppLogEntry>? subset}) {
    final list = subset ?? entries;
    return list.map((e) => e.formatLine()).join("\n");
  }

  /// Copy-safe export: newest lines first, capped at [maxExportBytes].
  ({String text, int lineCount, bool truncated}) exportTextBounded({
    Iterable<AppLogEntry>? subset,
    int maxBytes = maxExportBytes,
  }) {
    final list = (subset ?? entries).toList();
    if (list.isEmpty) {
      return (text: "", lineCount: 0, truncated: false);
    }
    final lines = <String>[];
    var bytes = 0; // UTF-8 bytes (Android clipboard/Binder limit is bytes, not code units)
    var truncated = false;
    final enc = const Utf8Encoder();
    for (var i = list.length - 1; i >= 0; i--) {
      final line = list[i].formatLine();
      final add = enc.convert(line).length + (lines.isEmpty ? 0 : 1); // + newline
      if (bytes + add > maxBytes && lines.isNotEmpty) {
        truncated = true;
        break;
      }
      lines.add(line);
      bytes += add;
    }
    if (!truncated && lines.length < list.length) {
      truncated = true;
    }
    final body = lines.reversed.join("\n");
    if (!truncated) {
      return (text: body, lineCount: lines.length, truncated: false);
    }
    final header =
        "### TRUNCATED (${lines.length} newest lines, limit=${maxBytes}B; use Share for full log)\n";
    // If header pushes us over, drop a few oldest lines from the bounded slice.
    var out = "$header$body";
    while (enc.convert(out).length > maxBytes && lines.length > 1) {
      lines.removeAt(0);
      final b2 = lines.reversed.join("\n");
      out = "$header$b2";
    }
    return (text: out, lineCount: lines.length, truncated: true);
  }
}

enum AppLogLevel { debug, info, warn, error }

extension AppLogLevelX on AppLogLevel {
  String get label => switch (this) {
    AppLogLevel.debug => "DBG",
    AppLogLevel.info => "INF",
    AppLogLevel.warn => "WRN",
    AppLogLevel.error => "ERR",
  };
}

class AppLogEntry {
  AppLogEntry({
    required this.at,
    required this.level,
    required this.tag,
    required this.message,
  });

  final DateTime at;
  final AppLogLevel level;
  final String tag;
  final String message;

  String formatLine() {
    final ts = at.toIso8601String();
    final ms = at.millisecond.toString().padLeft(3, "0");
    final t = "${ts.substring(0, 19)}.$ms";
    return "$t [${level.label}] [$tag] $message";
  }
}
