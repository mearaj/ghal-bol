/// Human-readable label: optional [customAlias], else short `public_key_hex` slice.
String ghalBolIdName({
  String? publicKeyHex,
  String? signingPublicKeyHex,
  String? customAlias,
}) {
  final c = customAlias?.trim();
  if (c != null && c.isNotEmpty) {
    return c;
  }
  final h = publicKeyHex?.trim() ?? signingPublicKeyHex?.trim() ?? "";
  final isHex = RegExp(r"^[0-9a-fA-F]+$").hasMatch(h);
  if (isHex && h.length >= 8) {
    final lo = h.toLowerCase();
    return "${lo.substring(0, 4)}..${lo.substring(lo.length - 4)}";
  }
  return "—";
}

/// Single-line display hint for invites; `null` means omit on the wire.
String? ghalSanitizePeerAlias(String? raw, {int maxLen = 64}) {
  if (raw == null) return null;
  var t = raw.trim();
  if (t.isEmpty) return null;
  t = t.replaceAll(RegExp(r"[\n\r\t]+"), " ");
  if (t.length > maxLen) {
    t = t.substring(0, maxLen).trimRight();
  }
  if (t.isEmpty) return null;
  return t;
}
