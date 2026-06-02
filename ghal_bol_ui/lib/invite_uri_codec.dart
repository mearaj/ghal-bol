/// Shared with [`GhalBolConnectInvite`] / Rust `connect_invite_v1`.
const String kGhalBolConnectShareV1 = "ghal_bol_connect_v1";
const int kConnectInviteFormatV2 = 2;

const String kGhalBolConnectHttpsHost = "ghalbol.com";
const String kGhalBolConnectAppScheme = "ghalbol";
const String kGhalBolConnectPathSegment = "connect";

const String _defaultTopic = "ghal-bol-chat";

String normalizeInviteInput(String input) => input.trim().replaceAll(RegExp(r"\s+"), "");

/// `https://ghalbol.com/connect/<public_key_hex>` — optional `?alias=` only.
String encodeConnectInviteUri(Map<String, dynamic> wire) {
  final pk = _publicKeyFromWire(wire);
  if (pk == null) {
    throw ArgumentError("wire map missing valid public_key_hex");
  }
  final alias = wire["peer_alias"]?.toString().trim();
  final base = "https://$kGhalBolConnectHttpsHost/$kGhalBolConnectPathSegment/$pk";
  return _appendInviteQuery(base, alias: alias);
}

/// `ghalbol://connect/<public_key_hex>` — optional `?alias=` only.
String encodeConnectInviteAppUri(Map<String, dynamic> wire) {
  final pk = _publicKeyFromWire(wire);
  if (pk == null) {
    throw ArgumentError("wire map missing valid public_key_hex");
  }
  final alias = wire["peer_alias"]?.toString().trim();
  final base = "$kGhalBolConnectAppScheme://$kGhalBolConnectPathSegment/$pk";
  return _appendInviteQuery(base, alias: alias);
}

String _appendInviteQuery(String base, {String? alias}) {
  final a = alias?.trim();
  if (a == null || a.isEmpty) return base;
  return "$base?alias=${Uri.encodeQueryComponent(a)}";
}

Map<String, dynamic>? decodeConnectInviteUri(String input) {
  final t = normalizeInviteInput(input);
  return _decodePathStyleInvite(t);
}

Map<String, dynamic>? _decodePathStyleInvite(String t) {
  final lower = t.toLowerCase();
  final qIdx = t.indexOf("?");
  final pathPart = qIdx >= 0 ? t.substring(0, qIdx) : t;
  final query = qIdx >= 0 ? t.substring(qIdx + 1) : null;
  final alias = _parseAliasQuery(query);

  final appPrefix = "$kGhalBolConnectAppScheme://";
  if (lower.startsWith(appPrefix)) {
    final pk = _pkFromPathRest(pathPart.substring(appPrefix.length));
    if (pk != null) return _wireFromPk(pk, alias: alias);
  }

  for (final scheme in ["https://", "http://"]) {
    final prefix = "$scheme$kGhalBolConnectHttpsHost/";
    if (lower.startsWith(prefix)) {
      final pk = _pkFromPathRest(pathPart.substring(prefix.length));
      if (pk != null) return _wireFromPk(pk, alias: alias);
    }
  }
  return null;
}

String? _pkFromPathRest(String rest) {
  var r = rest.trim();
  if (r.startsWith("$kGhalBolConnectPathSegment/")) {
    r = r.substring(kGhalBolConnectPathSegment.length + 1);
  } else if (r.startsWith(kGhalBolConnectPathSegment)) {
    r = r.substring(kGhalBolConnectPathSegment.length);
  }
  r = r.replaceAll("/", "").trim();
  return _isHex66(r) ? r.toLowerCase() : null;
}

String? _parseAliasQuery(String? query) {
  if (query == null || query.isEmpty) return null;
  for (final part in query.split("&")) {
    if (part.startsWith("alias=")) {
      final v = Uri.decodeQueryComponent(part.substring(6));
      final t = v.trim();
      if (t.isNotEmpty) return t;
    }
  }
  return null;
}

Map<String, dynamic> _wireFromPk(String pk, {String? alias}) {
  return {
    "ghalbol.share": kGhalBolConnectShareV1,
    "format_version": kConnectInviteFormatV2,
    "topic": _defaultTopic,
    "public_key_hex": pk,
    ...?(alias == null ? null : {"peer_alias": alias}),
  };
}

String? _publicKeyFromWire(Map<String, dynamic> wire) {
  final pk = wire["public_key_hex"]?.toString().trim().toLowerCase() ?? "";
  return _isHex66(pk) ? pk : null;
}

int? _formatVersion(Map<String, dynamic> v) {
  final raw = v["format_version"];
  if (raw is int) return raw;
  if (raw is num) return raw.toInt();
  return int.tryParse(raw?.toString() ?? "");
}

bool _fieldNonempty(Map<String, dynamic> v, String key) {
  final s = v[key]?.toString().trim() ?? "";
  return s.isNotEmpty;
}

bool _isHex66(String hex) {
  final s = hex.trim();
  if (s.length != 66) return false;
  if (!RegExp(r"^[0-9a-fA-F]+$").hasMatch(s)) return false;
  try {
    for (var i = 0; i < s.length; i += 2) {
      int.parse(s.substring(i, i + 2), radix: 16);
    }
    return true;
  } catch (_) {
    return false;
  }
}

bool verifyConnectInviteWireMap(Map<String, dynamic> v) {
  if (v["ghalbol.share"]?.toString() != kGhalBolConnectShareV1) return false;
  if (_formatVersion(v) != kConnectInviteFormatV2) return false;
  if (v["topic"]?.toString().trim().isEmpty ?? true) return false;
  if (_fieldNonempty(v, "peer_id")) return false;
  if (_fieldNonempty(v, "invite_signature_hex")) return false;
  if (_fieldNonempty(v, "encryption_public_key_hex")) return false;
  if (v.containsKey("multiaddrs")) return false;
  if (_fieldNonempty(v, "coord_base_url")) return false;
  final pk = v["public_key_hex"]?.toString() ?? "";
  return _isHex66(pk);
}

String? extractConnectInviteUri(String? raw) {
  if (raw == null) return null;
  final t = normalizeInviteInput(raw);
  if (t.isEmpty) return null;

  final https = RegExp(
    r"https?://ghalbol\.com/connect/[0-9a-fA-F]{66}[^\s]*",
    caseSensitive: false,
  ).firstMatch(t);
  if (https != null) return https.group(0);

  final app = RegExp(
    r"ghalbol://connect/[0-9a-fA-F]{66}[^\s]*",
    caseSensitive: false,
  ).firstMatch(t);
  if (app != null) return app.group(0);

  return null;
}
