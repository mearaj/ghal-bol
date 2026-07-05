import "dart:async";
import "dart:convert";
import "dart:io";

import "daemon_client_api.dart";

/// One serialized JSON-RPC line channel over a Unix domain socket.
class RpcConnection {
  RpcConnection();

  Socket? _socket;
  StreamIterator<String>? _lines;
  int _nextId = 1;
  Future<void> _chain = Future<void>.value();

  static Future<bool> pingSocket(String socketPath) async {
    Socket? s;
    try {
      s = await Socket.connect(
        InternetAddress(socketPath, type: InternetAddressType.unix),
        0,
      ).timeout(const Duration(seconds: 2));
      const id = 0;
      s.writeln(jsonEncode({"id": id, "method": DaemonMethod.ping, "params": {}}));
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

  Future<void> connect(String socketPath) {
    return _serialized(() async {
      if (_socket != null) return;
      final s = await Socket.connect(
        InternetAddress(socketPath, type: InternetAddressType.unix),
        0,
      );
      _socket = s;
      _lines = StreamIterator(
        s.cast<List<int>>().transform(utf8.decoder).transform(const LineSplitter()),
      );
    });
  }

  Future<void> disconnect() {
    return _serialized(() async {
      await _lines?.cancel();
      _lines = null;
      try {
        await _socket?.close();
      } catch (_) {}
      _socket = null;
    });
  }

  Future<Map<String, dynamic>> call(
    String method, {
    Map<String, dynamic> params = const {},
  }) {
    return _serialized(() async {
      if (_socket == null || _lines == null) {
        return {"ok": false, "error": "not connected"};
      }
      try {
        final id = _nextId++;
        _socket!.writeln(
          jsonEncode({"id": id, "method": method, "params": params}),
        );
        while (await _lines!.moveNext()) {
          final line = _lines!.current.trim();
          if (line.isEmpty) continue;
          final raw = jsonDecode(line);
          if (raw is! Map || raw["id"] != id) continue;
          if (raw["error"] != null) {
            return {"ok": false, "error": raw["error"].toString()};
          }
          final payload = raw["result"];
          if (payload is Map<String, dynamic>) return payload;
          if (payload is Map) return Map<String, dynamic>.from(payload);
          return {"ok": true, "result": payload};
        }
        await disconnect();
        return {"ok": false, "error": "daemon disconnected"};
      } catch (e) {
        await disconnect();
        return {"ok": false, "error": e.toString()};
      }
    });
  }

  Future<T> _serialized<T>(Future<T> Function() run) {
    final next = _chain.then((_) => run());
    _chain = next.then((_) {}, onError: (_) {});
    return next;
  }
}
