import "package:ghal_bol_ui/ghal_bol_ffi.dart";

/// Compressed secp256k1 public key on the wire (33 bytes → 66 hex chars).
const int kSecp256k1PublicKeyHexLen = 66;

bool isValidPublicKeyHex(String? hex) {
  final s = hex?.trim() ?? "";
  if (s.length != kSecp256k1PublicKeyHexLen) return false;
  return RegExp(r"^[0-9a-fA-F]+$").hasMatch(s);
}

bool publicKeysEqual(String? a, String? b) {
  final pa = a?.trim().toLowerCase() ?? "";
  final pb = b?.trim().toLowerCase() ?? "";
  return isValidPublicKeyHex(pa) && pa == pb;
}

/// libp2p PeerId string for transport only (derived from signing public key).
String? libp2pPeerIdFromPublicKeyHex(String? publicKeyHex) {
  final pk = publicKeyHex?.trim().toLowerCase() ?? "";
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

/// Resolve a 66-hex key from storage (authoritative).
String? resolvePublicKeyHex({String? storedHex}) {
  final stored = storedHex?.trim().toLowerCase() ?? "";
  if (isValidPublicKeyHex(stored)) return stored;
  return null;
}

/// Contact / roster / foreground identity from a native poll event — **public key only**.
String contactPublicKeyHexFromEvent(Map<String, dynamic> ev) {
  for (final key in <String>["sender_public_key_hex", "public_key_hex"]) {
    final s = ev[key]?.toString().trim().toLowerCase() ?? "";
    if (isValidPublicKeyHex(s)) return s;
  }
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
