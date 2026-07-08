import "package:ghal_bol_ui/ghal_bol_ffi.dart";

/// Contact identity wire helpers — validation via native `Identity::parse` (all algorithms).

bool isValidPublicKeyHex(String? hex) {
  final s = hex?.trim() ?? "";
  if (s.isEmpty) return false;
  return GhalBolFfi.identityParse(s)["ok"] == true;
}

bool publicKeysEqual(String? a, String? b) => GhalBolFfi.identitySame(a, b);

/// libp2p PeerId string for transport only (derived from signing public key).
String? libp2pPeerIdFromPublicKeyHex(String? publicKeyHex) {
  final pk = GhalBolFfi.identityNormalize(publicKeyHex) ?? "";
  if (!isValidPublicKeyHex(pk)) return null;
  return GhalBolFfi.peerIdFromPublicKeyHex(pk);
}

/// True when [wirePeerId] is the libp2p connection id for [contactPublicKeyHex].
bool libp2pWireMatchesContactPublicKey({
  required String? wirePeerId,
  required String contactPublicKeyHex,
}) {
  final wire = wirePeerId?.trim() ?? "";
  if (wire.isEmpty) return false;
  final derived = libp2pPeerIdFromPublicKeyHex(contactPublicKeyHex);
  return derived != null && derived == wire;
}

/// Preferred contact identity wire from unlock/session or storage field.
String? identityWireFromSession({
  String? identityWire,
  String? publicKeyHex,
  String? identityAlgorithm,
}) {
  for (final candidate in <String?>[
    identityWire,
    publicKeyHex,
    _prefixedIdentityWire(publicKeyHex, identityAlgorithm),
  ]) {
    final wire = resolvePublicKeyHex(storedHex: candidate);
    if (wire != null && isValidPublicKeyHex(wire)) return wire;
  }
  return null;
}

String? _prefixedIdentityWire(String? publicKeyHex, String? identityAlgorithm) {
  final pk = publicKeyHex?.trim() ?? "";
  final algo = identityAlgorithm?.trim() ?? "";
  if (pk.isEmpty || algo.isEmpty || algo == "secp256k1") return null;
  if (pk.contains(":")) return null;
  return "$algo:$pk";
}

/// Resolve identity wire from storage (authoritative).
String? resolvePublicKeyHex({String? storedHex}) =>
    GhalBolFfi.identityNormalize(storedHex);

/// Contact / roster / foreground identity from a native poll event — **public key only**.
String contactPublicKeyHexFromEvent(Map<String, dynamic> ev) {
  for (final key in <String>["sender_public_key_hex", "public_key_hex"]) {
    final s = ev[key]?.toString().trim() ?? "";
    final norm = GhalBolFfi.identityNormalize(s);
    if (norm != null && norm.isNotEmpty) return norm;
  }
  return "";
}

/// Contact key for stream-ready / call gating from a connect event.
/// Resolves libp2p `peer_id` → identity wire when the poll JSON only carried wire id.
String streamContactKeyFromEvent(Map<String, dynamic> ev) {
  final pk = contactPublicKeyHexFromEvent(ev);
  if (isValidPublicKeyHex(pk)) return pk;
  final wire = libp2pWirePeerIdFromEvent(ev);
  if (wire.isEmpty) return "";
  final resolved = GhalBolFfi.publicKeyHexFromPeerId(wire)?.trim() ?? "";
  final norm = GhalBolFfi.identityNormalize(resolved);
  if (norm != null && norm.isNotEmpty) return norm;
  return "";
}

/// libp2p transport id from event (`from` / `peer_id`) — not a contact key.
String libp2pWirePeerIdFromEvent(Map<String, dynamic> ev) {
  for (final key in <String>["from", "peer_id"]) {
    final s = ev[key]?.toString().trim() ?? "";
    if (s.isNotEmpty) return s;
  }
  return "";
}

/// Prefer [contactPublicKeyHexFromEvent]; falls back to native `public_key_hex` on connect events.
String publicKeyHexFromEvent(Map<String, dynamic> ev) {
  final pk = contactPublicKeyHexFromEvent(ev);
  if (pk.isNotEmpty) return pk;
  return "";
}

/// Contact / stream key from a native poll event (`public_key_hex` / `sender_public_key_hex` only).
String contactKeyFromEvent(Map<String, dynamic> ev) => contactPublicKeyHexFromEvent(ev);
