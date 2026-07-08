import "package:ghal_bol_ui/network_helper.dart";

/// Short user-facing P2P link errors. Full native text stays in [AppLog] only.
const int kMaxUserP2pErrorLen = 72;

bool isTransientP2pLinkError(String raw) {
  final r = raw.trim().toLowerCase();
  if (r.isEmpty) return true;
  if (r.contains("open chat stream")) return true;
  if (r.contains("chat stream opening")) return true;
  if (r.contains("chat stream not ready")) return true;
  if (r.contains("connecting to peer")) return true;
  if (r.contains("open_stream")) return true;
  if (r.contains("wait until connected")) return true;
  if (r.contains("try send again shortly")) return true;
  if (r.contains("transport kem not ready")) return true;
  if (r.contains("stream opening")) return true;
  if (r.contains("dialpeercondition")) return true;
  if (r.contains("no addresses")) return true;
  if (r.contains("not connected")) return true;
  if (r.contains("broken pipe")) return true;
  if (r.contains("connection reset")) return true;
  if (r.contains("write failed")) return true;
  if (r.contains("socketexception")) return true;
  if (r.contains("daemon disconnected")) return true;
  if (r.contains("daemon not running")) return true;
  if (r.contains("reconnecting")) return true;
  return false;
}

/// `null` = do not show an error banner (transient or empty).
String? shortUserP2pError(String raw) {
  final trimmed = raw.trim();
  if (trimmed.isEmpty || isTransientP2pLinkError(trimmed)) return null;

  final r = trimmed.toLowerCase();
  if (r.contains("peer not known") || r.contains("unknown contact")) {
    return "Add them via invitation first.";
  }
  if (r.contains("broken pipe") ||
      r.contains("connection reset") ||
      r.contains("write failed") ||
      r.contains("socketexception") ||
      r.contains("daemon disconnected") ||
      r.contains("daemon not running")) {
    return null;
  }
  if (r.contains("connection refused") ||
      r.contains("timeout") ||
      r.contains("failed to negotiate transport")) {
    return "Peer not reachable — keep both apps open.";
  }
  if (r.contains("p2p not running") || r.contains("identity not unlocked")) {
    return "Network not ready — unlock and wait a moment.";
  }
  if (r.contains("own device")) {
    return "This is your own device.";
  }

  var s = trimmed;
  for (final prefix in <String>[
    "open chat stream:",
    "failed to open chat stream:",
  ]) {
    if (s.toLowerCase().startsWith(prefix)) {
      s = s.substring(prefix.length).trim();
      if (isTransientP2pLinkError(s)) return null;
      break;
    }
  }

  if (s.length > kMaxUserP2pErrorLen) {
    return "${s.substring(0, kMaxUserP2pErrorLen - 1)}…";
  }
  return s;
}

/// Prefer OS offline hint when we have a live probe and a non-transient P2P error.
String? networkAwareUserP2pError(String raw) {
  final snap = NetworkHelper.instance.snapshot.value;
  if (snap.hasLiveSnapshot && snap.appearsOffline) {
    return "No internet connection.";
  }
  return shortUserP2pError(raw);
}
