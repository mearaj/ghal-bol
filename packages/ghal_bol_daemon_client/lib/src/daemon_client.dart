import "daemon_client_api.dart";
import "integrator_config.dart";
import "rpc_connection.dart";

/// High-level daemon SDK client (main + state sockets).
class DaemonClient {
  DaemonClient._(this.config, this._main, this._state);

  final IntegratorConfig config;
  final RpcConnection _main;
  final RpcConnection _state;

  static Future<DaemonClient> connect(
    IntegratorConfig config, {
    required String socketPath,
  }) async {
    final main = RpcConnection();
    final state = RpcConnection();
    await main.connect(socketPath);
    await state.connect(socketPath);
    return DaemonClient._(config, main, state);
  }

  Future<void> disconnect() async {
    await _main.disconnect();
    await _state.disconnect();
  }

  Future<Map<String, dynamic>> call(
    String method, {
    Map<String, dynamic> params = const {},
  }) =>
      _main.call(method, params: params);

  Future<Map<String, dynamic>> callState(
    String method, {
    Map<String, dynamic> params = const {},
  }) =>
      _state.call(method, params: params);

  Future<bool> ping() async {
    final r = await call(DaemonMethod.ping);
    return r["pong"] == true;
  }

  Future<Map<String, dynamic>> unlock(String password) => call(
        DaemonMethod.unlock,
        params: {
          "app_namespace": config.appNamespace,
          "password": password,
        },
      );

  Future<bool> sessionUnlocked() async {
    final r = await call(DaemonMethod.sessionUnlocked);
    return r["ok"] == true && r["unlocked"] == true;
  }

  Future<Map<String, dynamic>?> pollEvent() async {
    final r = await callState(DaemonMethod.p2pPoll);
    if (r["ok"] != true) return null;
    final ev = r["event"];
    if (ev == null) return null;
    if (ev is Map<String, dynamic>) return ev;
    if (ev is Map) return Map<String, dynamic>.from(ev);
    return null;
  }

  Future<Map<String, dynamic>> syncUiSession({
    required bool uiVisible,
    String? roomPublicKeyHex,
  }) {
    final params = <String, dynamic>{"ui_visible": uiVisible};
    final pk = roomPublicKeyHex?.trim() ?? "";
    if (pk.isNotEmpty) params["room_public_key_hex"] = pk;
    return callState(DaemonMethod.p2pSyncUiSession, params: params);
  }

  Future<bool> takeUnlockWake() async {
    final r = await callState(DaemonMethod.p2pTakeUnlockWake);
    return r["wake"] == true;
  }

  Future<bool> takeIncomingCallWake() async {
    final r = await callState(DaemonMethod.p2pTakeIncomingCallWake);
    return r["wake"] == true;
  }

  Future<void> prepareReconnect({int suppressMs = 5000}) async {
    await call(
      DaemonMethod.uiSessionPrepareReconnect,
      params: {"suppress_ms": suppressMs},
    );
  }

  Future<void> notifyUiProcessExiting() async {
    await call(DaemonMethod.uiProcessExiting);
  }
}
