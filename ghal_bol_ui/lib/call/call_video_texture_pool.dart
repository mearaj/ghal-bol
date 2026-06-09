import "package:ghal_bol_ui/call/call_video_texture_bridge.dart";

/// One GPU texture per `(call_id, track)` — avoids duplicate register storms on rebuild.
class CallVideoTexturePool {
  CallVideoTexturePool._();

  static final _entries = <String, _Entry>{};
  static final _refs = <String, int>{};

  static String _key(String callId, String track) => "$callId::$track";

  static ({int id, int w, int h})? peek(String callId, String track) {
    final e = _entries[_key(callId, track)];
    if (e == null) return null;
    return (id: e.id, w: e.w, h: e.h);
  }

  static void retain(String callId, String track, int id, int w, int h) {
    final key = _key(callId, track);
    _entries[key] = _Entry(id: id, w: w, h: h);
    _refs[key] = (_refs[key] ?? 0) + 1;
  }

  /// Intentionally a no-op: textures are released only on [releaseCall] (hangup).
  /// Releasing on widget dispose/rebuild caused Android crashes during PiP swap.
  static void releaseWidget(String callId, String track) {}

  static Future<void> releaseCall(String callId) async {
    final prefix = "$callId::";
    final keys = _entries.keys.where((k) => k.startsWith(prefix)).toList();
    for (final key in keys) {
      final e = _entries.remove(key);
      _refs.remove(key);
      if (e != null) {
        await CallVideoTextureBridge.release(e.id);
      }
    }
  }

  static Future<void> releaseAll() async {
    for (final e in _entries.values) {
      await CallVideoTextureBridge.release(e.id);
    }
    _entries.clear();
    _refs.clear();
    await CallVideoTextureBridge.releaseAll();
  }
}

class _Entry {
  _Entry({required this.id, required this.w, required this.h});
  final int id;
  final int w;
  final int h;
}
