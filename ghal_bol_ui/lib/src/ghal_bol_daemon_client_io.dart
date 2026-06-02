import "dart:async";
import "dart:convert";
import "dart:io";

import "package:ghal_bol_ui/app_log.dart";
import "package:ghal_bol_ui/user_flow_log.dart";
import "package:ghal_bol_ui/ghal_bol_ffi.dart";
import "package:ghal_bol_ui/src/ghal_bol_android_p2p_service.dart"
    if (dart.library.html) "package:ghal_bol_ui/src/ghal_bol_android_p2p_service_stub.dart";

/// Unix-socket client for out-of-process P2P (`ghal_bol_daemon` on Linux, `:p2p` service on Android).
class GhalBolDaemonClient {
  GhalBolDaemonClient._();

  static final GhalBolDaemonClient instance = GhalBolDaemonClient._();

  Socket? _socket;
  StreamIterator<String>? _lines;
  int _nextId = 1;
  /// Separate socket for room/ack gates — must not queue behind `send_text_dm` / poll.
  Socket? _stateSocket;
  StreamIterator<String>? _stateLines;
  int _stateNextId = 1;
  String? _cachedSocketPath;

  /// One in-flight RPC at a time per socket (line iterator is not multiplexed).
  Future<void> _rpcChain = Future<void>.value();
  Future<void> _stateRpcChain = Future<void>.value();

  static bool get _usesOutOfProcessP2p =>
      Platform.isLinux || Platform.isAndroid;

  static Future<String> resolveSocketPath() async {
    if (Platform.isAndroid) {
      return ghalBolAndroidP2pSocketPath();
    }
    final fromEnv = Platform.environment["GHAL_BOL_DAEMON_SOCKET"]?.trim();
    if (fromEnv != null && fromEnv.isNotEmpty) return fromEnv;
    final fromNative = GhalBolFfi.daemonSocketPath();
    if (fromNative != null && fromNative.isNotEmpty) return fromNative;
    return "/tmp/ghalbol/p2p.sock";
  }

  Future<String> _socketPath() async {
    _cachedSocketPath ??= await resolveSocketPath();
    return _cachedSocketPath!;
  }

  static Future<String?> resolveDaemonExecutable() async {
    if (!Platform.isLinux) return null;
    final exeDir = File(Platform.resolvedExecutable).parent;
    final bundled = File("${exeDir.path}/libexec/ghal_bol_daemon");
    if (await bundled.exists()) return bundled.path;
    final dev = File("${exeDir.path}/../../../../target/debug/ghal_bol_daemon");
    if (await dev.exists()) return dev.absolute.path;
    final devRelease =
        File("${exeDir.path}/../../../../target/release/ghal_bol_daemon");
    if (await devRelease.exists()) return devRelease.absolute.path;
    return null;
  }

  static Future<bool> _pingSocket(String path) async {
    Socket? s;
    try {
      s = await Socket.connect(
        InternetAddress(path, type: InternetAddressType.unix),
        0,
      ).timeout(const Duration(seconds: 2));
      final id = 0;
      s.writeln(jsonEncode({"id": id, "method": "ping", "params": {}}));
      final lines = s
          .cast<List<int>>()
          .transform(utf8.decoder)
          .transform(const LineSplitter());
      await for (final line in lines) {
        if (line.trim().isEmpty) continue;
        final raw = jsonDecode(line);
        if (raw is Map && raw["id"] == id) {
          return raw["result"]?["pong"] == true;
        }
        break;
      }
      return false;
    } catch (_) {
      return false;
    } finally {
      await s?.close();
    }
  }

  static DateTime? _probeOkAt;
  static const Duration _probeTtl = Duration(seconds: 60);

  static void invalidateProbeCache() {
    _probeOkAt = null;
  }

  /// Cached reachability — do not open a new ping socket on every [p2p_poll] (was ~5×/sec).
  static Future<bool> probeDaemon({bool force = false}) async {
    if (!_usesOutOfProcessP2p) return false;
    if (!force &&
        _probeOkAt != null &&
        DateTime.now().difference(_probeOkAt!) < _probeTtl) {
      return true;
    }
    if (Platform.isLinux) {
      final bin = await resolveDaemonExecutable();
      if (bin != null) {
        final r = await Process.run(bin, ["--probe"]);
        if (r.exitCode == 0) return true;
      }
    }
    try {
      final path = await instance._socketPath();
      final ok = await _pingSocket(path);
      if (ok) {
        _probeOkAt = DateTime.now();
      } else {
        invalidateProbeCache();
      }
      return ok;
    } catch (_) {
      invalidateProbeCache();
      return false;
    }
  }

  static Future<void>? _ensureDaemonChain;

  static Future<void> ensureDaemonRunning() async {
    if (!_usesOutOfProcessP2p) return;
    final run = _ensureDaemonRunningOnce;
    final next = (_ensureDaemonChain ?? Future<void>.value()).then((_) => run());
    _ensureDaemonChain = next.catchError((_) {});
    return next;
  }

  /// Drop stale UI RPC sockets only — does **not** stop the `:p2p` process (keeps libp2p up).
  static Future<void> reconnectDaemon() async {
    if (!_usesOutOfProcessP2p) return;
    await instance.disconnect();
    invalidateProbeCache();
    await ensureDaemonRunning();
    for (var i = 0; i < 40; i++) {
      await Future<void>.delayed(const Duration(milliseconds: 100));
      if (await probeDaemon(force: true)) {
        SessionFlowLog.daemon("reconnect_ok");
        return;
      }
    }
    SessionFlowLog.daemonIssue("reconnect_failed");
  }

  /// Drop stale UI sockets and bring `:p2p` / `ghal_bol_daemon` back before unlock.
  static Future<void> prepareForLoginUnlock() async {
    if (!_usesOutOfProcessP2p) return;
    SessionFlowLog.daemon("prepare_login", {"action": "disconnect_sockets"});
    await reconnectDaemon();
    if (!await probeDaemon(force: true)) {
      SessionFlowLog.daemon("prepare_login_hard_reset");
      await hardResetP2pService();
    } else {
      SessionFlowLog.daemon("prepare_login_ok");
    }
  }

  /// Stop and restart Android `:p2p` — last resort (drops active libp2p until unlock + p2p_start).
  static Future<void> hardResetP2pService() async {
    if (!_usesOutOfProcessP2p) return;
    await instance.disconnect();
    invalidateProbeCache();
    if (Platform.isAndroid) {
      try {
        await ghalBolAndroidStopP2pService();
        await Future<void>.delayed(const Duration(milliseconds: 400));
      } catch (e) {
        AppLog.instance.w("Daemon", "stop :p2p: $e");
      }
      instance._cachedSocketPath = await ghalBolAndroidStartP2pService();
    }
    for (var i = 0; i < 50; i++) {
      await Future<void>.delayed(const Duration(milliseconds: 100));
      if (await probeDaemon(force: true)) {
        SessionFlowLog.daemon("hard_reset_ok");
        return;
      }
    }
    SessionFlowLog.daemonIssue("hard_reset_failed");
  }

  /// Reconnect UI sockets; only hard-reset `:p2p` if the daemon still does not answer ping.
  static Future<void> forceRecoverDaemon() async {
    if (!_usesOutOfProcessP2p) return;
    await reconnectDaemon();
    if (await probeDaemon(force: true)) return;
    await hardResetP2pService();
  }

  static bool _isRecoverableDaemonError(String? err) {
    if (err == null || err.isEmpty) return false;
    final low = err.toLowerCase();
    return low.contains("disconnected") ||
        low.contains("not running") ||
        low.contains("broken pipe") ||
        low.contains("connection reset") ||
        low.contains("connection refused") ||
        low.contains("timed out");
  }

  /// Unlock with `:p2p` restart on transient socket failures.
  ///
  /// Does **not** call [prepareForLoginUnlock] on the first attempt — login and
  /// UI-lock flows must call that once before FFI unlock. Re-unlock via [unlock]
  /// must not disconnect sockets mid-session.
  Future<Map<String, dynamic>> unlockWithRecovery({
    required String appNamespace,
    required String password,
  }) async {
    Map<String, dynamic>? last;
    for (var attempt = 0; attempt < 3; attempt++) {
      if (attempt == 1) {
        SessionFlowLog.daemon("unlock_retry", {"attempt": "1", "mode": "reconnect"});
        await reconnectDaemon();
      } else if (attempt == 2) {
        SessionFlowLog.daemon("unlock_retry", {"attempt": "2", "mode": "hard_reset"});
        await hardResetP2pService();
      }
      last = await unlock(appNamespace: appNamespace, password: password);
      if (last["ok"] == true) return last;
      if (!_isRecoverableDaemonError(last["error"]?.toString())) {
        return last;
      }
    }
    return last ?? {"ok": false, "error": "daemon unlock failed"};
  }

  static Future<void> _ensureDaemonRunningOnce() async {
    if (await probeDaemon()) {
      SessionFlowLog.daemon("ensure_running", {"state": "already_up"});
      return;
    }

    SessionFlowLog.daemon("ensure_running", {"state": "starting"});
    if (Platform.isAndroid) {
      instance._cachedSocketPath = await ghalBolAndroidStartP2pService();
    } else if (Platform.isLinux) {
      if (await probeDaemon()) return;
      final bin = await resolveDaemonExecutable();
      if (bin == null) {
        SessionFlowLog.daemonIssue(
          "daemon_binary_missing",
          check: "run sync_ghal_bol_native_for_flutter.sh",
        );
        AppLog.instance.e("Daemon", "ghal_bol_daemon binary not found");
        return;
      }
      SessionFlowLog.daemon("spawn_daemon", {"bin": bin});
      await Process.start(
        bin,
        [],
        mode: ProcessStartMode.detached,
        environment: {
          if (Platform.environment["GHAL_BOL_DAEMON_SOCKET"] != null)
            "GHAL_BOL_DAEMON_SOCKET":
                Platform.environment["GHAL_BOL_DAEMON_SOCKET"]!,
        },
      );
    }

    for (var i = 0; i < 50; i++) {
      await Future<void>.delayed(const Duration(milliseconds: 100));
      if (await probeDaemon()) {
        final path = await instance._socketPath();
        SessionFlowLog.daemon("daemon_ready", {"socket": path});
        return;
      }
    }
    SessionFlowLog.daemonIssue(
      "daemon_start_timeout",
      check: "Android :p2p service; Linux ghal_bol_daemon in libexec",
    );
  }

  Future<void> _connect() async {
    if (_socket != null) return;
    final path = await _socketPath();
    final s = await Socket.connect(
      InternetAddress(path, type: InternetAddressType.unix),
      0,
    );
    _socket = s;
    _lines = StreamIterator(
      s.cast<List<int>>().transform(utf8.decoder).transform(const LineSplitter()),
    );
  }

  Future<void> _connectState() async {
    if (_stateSocket != null) return;
    final path = await _socketPath();
    final s = await Socket.connect(
      InternetAddress(path, type: InternetAddressType.unix),
      0,
    );
    _stateSocket = s;
    _stateLines = StreamIterator(
      s.cast<List<int>>().transform(utf8.decoder).transform(const LineSplitter()),
    );
  }

  Future<void> _resetConnection() async {
    await _lines?.cancel();
    _lines = null;
    try {
      await _socket?.close();
    } catch (_) {}
    _socket = null;
    invalidateProbeCache();
  }

  Future<void> _resetStateConnection() async {
    await _stateLines?.cancel();
    _stateLines = null;
    try {
      await _stateSocket?.close();
    } catch (_) {}
    _stateSocket = null;
    invalidateProbeCache();
  }

  Future<void> disconnect() async {
    await _resetConnection();
    await _resetStateConnection();
  }

  Future<T> _serialized<T>(Future<T> Function() run, {required bool state}) {
    final chain = state ? _stateRpcChain : _rpcChain;
    final next = chain.then((_) => run());
    if (state) {
      _stateRpcChain = next.then((_) {}, onError: (_) {});
    } else {
      _rpcChain = next.then((_) {}, onError: (_) {});
    }
    return next;
  }

  Future<Map<String, dynamic>> _callOnSocket({
    required String method,
    required Map<String, dynamic> params,
    required bool ensureDaemon,
    required bool stateSocket,
  }) {
    Future<Map<String, dynamic>> run() async {
      final sw = Stopwatch()..start();
      Map<String, dynamic> result;
      if (ensureDaemon) {
        await ensureDaemonRunning();
      } else if (!await probeDaemon()) {
        result = {"ok": false, "error": "daemon not running"};
        sw.stop();
        _logRpcResult(method, result, sw.elapsedMilliseconds, stateSocket: stateSocket);
        return result;
      }
      try {
        if (stateSocket) {
          await _connectState();
        } else {
          await _connect();
        }
        final socket = stateSocket ? _stateSocket! : _socket!;
        final lines = stateSocket ? _stateLines! : _lines!;
        final id = stateSocket ? _stateNextId++ : _nextId++;
        final req = jsonEncode({"id": id, "method": method, "params": params});
        socket.writeln(req);
        while (await lines.moveNext()) {
          final line = lines.current.trim();
          if (line.isEmpty) continue;
          final raw = jsonDecode(line);
          if (raw is! Map) continue;
          if (raw["id"] != id) continue;
          if (raw["error"] != null) {
            result = {"ok": false, "error": raw["error"].toString()};
            sw.stop();
            _logRpcResult(method, result, sw.elapsedMilliseconds, stateSocket: stateSocket);
            return result;
          }
          final payload = raw["result"];
          if (payload is Map<String, dynamic>) {
            result = payload;
          } else if (payload is Map) {
            result = Map<String, dynamic>.from(payload);
          } else {
            result = {"ok": true, "result": payload};
          }
          sw.stop();
          _logRpcResult(method, result, sw.elapsedMilliseconds, stateSocket: stateSocket);
          return result;
        }
        if (stateSocket) {
          await _resetStateConnection();
        } else {
          await _resetConnection();
        }
        result = {"ok": false, "error": "daemon disconnected"};
        sw.stop();
        _logRpcResult(method, result, sw.elapsedMilliseconds, stateSocket: stateSocket);
        return result;
      } catch (e) {
        if (stateSocket) {
          await _resetStateConnection();
        } else {
          await _resetConnection();
        }
        result = {"ok": false, "error": e.toString()};
        sw.stop();
        _logRpcResult(method, result, sw.elapsedMilliseconds, stateSocket: stateSocket);
        return result;
      }
    }
    return _serialized(run, state: stateSocket);
  }

  static DateTime? _lastPollRpcFailureLogAt;

  void _logRpcResult(
    String method,
    Map<String, dynamic> result,
    int elapsedMs, {
    required bool stateSocket,
  }) {
    if (method == "ping") return;
    final ok = result["ok"] == true;
    final err = result["error"]?.toString();
    if (method == "p2p_poll" || method == "p2p_is_running") {
      if (!ok) {
        final now = DateTime.now();
        final last = _lastPollRpcFailureLogAt;
        if (last == null || now.difference(last).inSeconds >= 30) {
          _lastPollRpcFailureLogAt = now;
          AppLog.instance.w("Daemon", "$method FAIL ${err ?? ""}");
        }
      }
      return;
    }
    AppLog.instance.rpc(
      "Daemon",
      method,
      ok: ok,
      error: err,
      elapsedMs: elapsedMs,
      stateSocket: stateSocket,
    );
    if (!ok && err != null) {
      final low = err.toLowerCase();
      if (low.contains("broken pipe") ||
          low.contains("connection reset") ||
          low.contains("daemon disconnected")) {
        AppLog.instance.w(
          "Daemon",
          "socket lost during $method — UI may need unlock+p2p_start to refresh handler context",
        );
      }
    }
    if (method == "unlock" && ok) {
      AppLog.instance.trace(
        "daemon_unlock",
        "ns=${result["app_namespace"]} pk=${_shortPk(result["public_key_hex"])}",
      );
    }
    if (method == "p2p_start" && ok) {
      AppLog.instance.trace(
        "p2p_start",
        "already_running=${result["already_running"] == true}",
      );
    }
  }

  static String _shortPk(Object? v) {
    final s = v?.toString().trim() ?? "";
    if (s.length <= 16) return s.isEmpty ? "(none)" : s;
    return "${s.substring(0, 8)}…";
  }

  Future<Map<String, dynamic>> call(
    String method, {
    Map<String, dynamic> params = const {},
    bool ensureDaemon = true,
  }) =>
      _callOnSocket(
        method: method,
        params: params,
        ensureDaemon: ensureDaemon,
        stateSocket: false,
      );

  /// Foreground peer + ack-read gate — never blocked behind chat send / poll.
  Future<Map<String, dynamic>> callState(
    String method, {
    Map<String, dynamic> params = const {},
    bool ensureDaemon = true,
  }) =>
      _callOnSocket(
        method: method,
        params: params,
        ensureDaemon: ensureDaemon,
        stateSocket: true,
      );

  Future<Map<String, dynamic>> unlock({
    required String appNamespace,
    required String password,
  }) =>
      call(
        "unlock",
        params: {
          "app_namespace": appNamespace,
          "password": password,
        },
        ensureDaemon: false,
      );

  Future<void> stopSession() async {
    await call("p2p_stop");
    await call("lock");
    await disconnect();
    _cachedSocketPath = null;
    if (Platform.isAndroid) {
      await ghalBolAndroidStopP2pService();
    }
  }
}
