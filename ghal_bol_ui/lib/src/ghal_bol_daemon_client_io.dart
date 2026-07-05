import "dart:async";
import "dart:io";

import "package:ghal_bol_ui/app_env_config.dart";
import "package:ghal_bol_ui/daemon_client_api.dart";
import "package:ghal_bol_ui/src/daemon_integrator_config.dart";
import "package:ghal_bol_ui/src/daemon_rpc_connection.dart";
import "package:ghal_bol_ui/app_log.dart";
import "package:ghal_bol_ui/ghal_bol_constants.dart";
import "package:ghal_bol_ui/user_flow_log.dart";
import "package:ghal_bol_ui/src/ghal_bol_android_p2p_service.dart"
    if (dart.library.html) "package:ghal_bol_ui/src/ghal_bol_android_p2p_service_stub.dart";

/// Reference integrator shell around [`DaemonClient`] / [`RpcConnection`].
///
/// Platform spawn (Linux `ghal_bol_daemon`, Android `:p2p` FGS) stays here.
/// Wire names: `daemon_client_api.dart`; Rust SDK: `ghal_bol::daemon`.
class GhalBolDaemonClient {
  GhalBolDaemonClient._();

  static final GhalBolDaemonClient instance = GhalBolDaemonClient._();

  final RpcConnection _main = RpcConnection();
  final RpcConnection _state = RpcConnection();
  String? _cachedSocketPath;

  static bool get _usesOutOfProcessP2p =>
      Platform.isLinux || Platform.isAndroid;

  static IntegratorConfig _integratorConfig({String? appNamespace}) {
    final ns = appNamespace?.trim().isNotEmpty == true
        ? appNamespace!.trim()
        : kGhalBolAppNamespace;
    return IntegratorConfig(
      appNamespace: ns,
      xdgRuntimeDir: Platform.environment["XDG_RUNTIME_DIR"],
      socketPathOverride: Platform.environment["GHAL_BOL_DAEMON_SOCKET"]?.trim(),
      runtimeDirOverride: Platform.environment["GHAL_BOL_RUNTIME_DIR"]?.trim(),
    );
  }

  static Future<String> resolveSocketPath({String? appNamespace}) async {
    if (Platform.isAndroid) {
      return ghalBolAndroidP2pSocketPath();
    }
    return _integratorConfig(appNamespace: appNamespace).socketPath;
  }

  Future<String> _socketPath() async {
    _cachedSocketPath ??= await resolveSocketPath(
      appNamespace: kGhalBolAppNamespace,
    );
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

  static DateTime? _probeOkAt;
  static const Duration _probeTtl = Duration(seconds: 60);

  static void invalidateProbeCache() {
    _probeOkAt = null;
  }

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
      final ok = await RpcConnection.pingSocket(path);
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

  static Future<void> reconnectDaemon() async {
    if (!_usesOutOfProcessP2p) return;
    try {
      await instance.call(
        DaemonMethod.uiSessionPrepareReconnect,
        params: {"suppress_ms": 5000},
        ensureDaemon: false,
      );
    } catch (_) {}
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

  static Future<void> prepareForLoginUnlock() async {
    if (!_usesOutOfProcessP2p) return;
    SessionFlowLog.daemon("prepare_login", {"action": "disconnect_sockets"});
    await reconnectDaemon();
    if (await probeDaemon(force: true)) {
      SessionFlowLog.daemon("prepare_login_ok");
    } else {
      SessionFlowLog.daemonIssue("prepare_login_probe_failed");
    }
  }

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

  Future<Map<String, dynamic>> unlockWithRecovery({
    required String appNamespace,
    required String password,
  }) async {
    Map<String, dynamic>? last;
    for (var attempt = 0; attempt < 3; attempt++) {
      if (attempt > 0) {
        SessionFlowLog.daemon("unlock_retry", {"attempt": attempt.toString(), "mode": "reconnect"});
        await reconnectDaemon();
      }
      last = await unlock(appNamespace: appNamespace, password: password);
      if (last["ok"] == true) return last;
      if (!_isRecoverableDaemonError(last["error"]?.toString())) {
        return last;
      }
    }
    return last ?? {"ok": false, "error": "daemon unlock failed"};
  }

  static bool _loggedDaemonAlreadyUp = false;

  static Future<void> _ensureDaemonRunningOnce() async {
    if (await probeDaemon()) {
      if (!_loggedDaemonAlreadyUp) {
        _loggedDaemonAlreadyUp = true;
        SessionFlowLog.daemon("ensure_running", {"state": "already_up"});
      }
      return;
    }
    _loggedDaemonAlreadyUp = false;

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
      final spawnEnv = Map<String, String>.from(Platform.environment);
      spawnEnv.addAll(_integratorConfig().daemonSpawnEnv());
      spawnEnv.addAll(_verboseLogEnv() ?? const {});
      await Process.start(
        bin,
        [],
        mode: ProcessStartMode.detached,
        environment: spawnEnv,
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

  static Map<String, String>? _verboseLogEnv() {
    final v = AppEnvConfig.get("GHAL_BOL_VERBOSE_LOG")?.trim() ?? "";
    if (v.isEmpty) return null;
    return {"GHAL_BOL_VERBOSE_LOG": v};
  }

  Future<void> _ensureConnected({required bool state}) async {
    final path = await _socketPath();
    if (state) {
      await _state.connect(path);
    } else {
      await _main.connect(path);
    }
  }

  Future<void> disconnect() async {
    await _main.disconnect();
    await _state.disconnect();
  }

  Future<Map<String, dynamic>> _callOnSocket({
    required String method,
    required Map<String, dynamic> params,
    required bool ensureDaemon,
    required bool stateSocket,
  }) async {
    final sw = Stopwatch()..start();
    if (ensureDaemon) {
      await ensureDaemonRunning();
    } else if (!await probeDaemon()) {
      final result = {"ok": false, "error": "daemon not running"};
      sw.stop();
      _logRpcResult(method, result, sw.elapsedMilliseconds, stateSocket: stateSocket);
      return result;
    }
    try {
      await _ensureConnected(state: stateSocket);
    } catch (e) {
      final result = {"ok": false, "error": e.toString()};
      sw.stop();
      _logRpcResult(method, result, sw.elapsedMilliseconds, stateSocket: stateSocket);
      return result;
    }
    final conn = stateSocket ? _state : _main;
    final result = await conn.call(method, params: params);
    sw.stop();
    _logRpcResult(method, result, sw.elapsedMilliseconds, stateSocket: stateSocket);
    return result;
  }

  static DateTime? _lastPollRpcFailureLogAt;
  static DateTime? _lastVideoFrameRpcLogAt;
  static DateTime? _lastCameraPushRpcLogAt;

  void _logRpcResult(
    String method,
    Map<String, dynamic> result,
    int elapsedMs, {
    required bool stateSocket,
  }) {
    if (method == DaemonMethod.ping) return;
    final ok = result["ok"] == true;
    final err = result["error"]?.toString();
    if (method == DaemonMethod.p2pPoll ||
        method == DaemonMethod.p2pIsRunning ||
        method == DaemonMethod.p2pTakeIncomingCallWake ||
        method == DaemonMethod.p2pTakeUnlockWake ||
        method == DaemonMethod.networkSnapshot) {
      if (!ok &&
          (method == DaemonMethod.p2pPoll || method == DaemonMethod.p2pIsRunning)) {
        final now = DateTime.now();
        final last = _lastPollRpcFailureLogAt;
        if (last == null || now.difference(last).inSeconds >= 30) {
          _lastPollRpcFailureLogAt = now;
          AppLog.instance.w("Daemon", "$method FAIL ${err ?? ""}");
        }
      }
      return;
    }
    if (method == DaemonMethod.p2pCallVideoFrame ||
        method == DaemonMethod.p2pCallVideoPushCameraFrame) {
      final now = DateTime.now();
      final last = method == DaemonMethod.p2pCallVideoPushCameraFrame
          ? _lastCameraPushRpcLogAt
          : _lastVideoFrameRpcLogAt;
      if (ok && last != null && now.difference(last).inMilliseconds < 1000) {
        return;
      }
      if (method == DaemonMethod.p2pCallVideoPushCameraFrame) {
        _lastCameraPushRpcLogAt = now;
      } else {
        _lastVideoFrameRpcLogAt = now;
      }
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
    if (method == DaemonMethod.unlock && ok) {
      AppLog.instance.trace(
        "daemon_unlock",
        "ns=${result["app_namespace"]} pk=${_shortPk(result["public_key_hex"])}",
      );
    }
    if (method == DaemonMethod.p2pStart && ok) {
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
        DaemonMethod.unlock,
        params: {
          "app_namespace": appNamespace,
          "password": password,
        },
        ensureDaemon: false,
      );

  Future<void> stopSession() async {
    await call(DaemonMethod.p2pStop);
    await call(DaemonMethod.lock);
    await disconnect();
    _cachedSocketPath = null;
    if (Platform.isAndroid) {
      await ghalBolAndroidStopP2pService();
    }
  }

  static Future<void> touchLinuxUiPresence({String? appNamespace}) async {
    if (!Platform.isLinux) return;
    try {
      final file = File(_integratorConfig(appNamespace: appNamespace).uiPresencePath);
      await file.parent.create(recursive: true);
      await file.writeAsString("1");
    } catch (_) {}
  }

  static Future<void> clearLinuxUiPresence({String? appNamespace}) async {
    if (!Platform.isLinux) return;
    try {
      final file = File(_integratorConfig(appNamespace: appNamespace).uiPresencePath);
      if (await file.exists()) await file.delete();
    } catch (_) {}
  }

  static Future<void> installLinuxAutostart() async {
    if (!Platform.isLinux) return;
    final bin = await resolveDaemonExecutable();
    if (bin == null) return;
    try {
      final home = Platform.environment["HOME"];
      if (home == null || home.isEmpty) return;
      final dir = Directory("$home/.config/autostart");
      if (!await dir.exists()) await dir.create(recursive: true);
      final file = File("${dir.path}/com.ghalbol.daemon.desktop");
      await file.writeAsString(
        "[Desktop Entry]\n"
        "Type=Application\n"
        "Name=Ghal Bol Background\n"
        "Exec=$bin\n"
        "Environment=GHAL_BOL_APP_NAMESPACE=$kGhalBolAppNamespace\n"
        "NoDisplay=true\n"
        "X-GNOME-Autostart-enabled=true\n"
        "Comment=Keeps Ghal Bol P2P networking active after login\n",
      );
      AppLog.instance.d("Daemon", "XDG autostart installed: ${file.path}");
    } catch (e) {
      AppLog.instance.w("Daemon", "XDG autostart install failed: $e");
    }
  }

  static Future<void> removeLinuxAutostart() async {
    if (!Platform.isLinux) return;
    try {
      final home = Platform.environment["HOME"];
      if (home == null || home.isEmpty) return;
      final file = File("$home/.config/autostart/com.ghalbol.daemon.desktop");
      if (await file.exists()) {
        await file.delete();
        AppLog.instance.d("Daemon", "XDG autostart removed");
      }
    } catch (e) {
      AppLog.instance.w("Daemon", "XDG autostart remove failed: $e");
    }
  }
}
