import "dart:convert";

import "package:ghal_bol_ui/ghal_bol_ffi.dart";
import "identity_display_name.dart";
import "invite_uri_codec.dart";
import "public_key_hex.dart" show isValidPublicKeyHex, resolvePublicKeyHex;

export "invite_uri_codec.dart"
    show
        encodeConnectInviteAppUri,
        extractConnectInviteUri,
        kGhalBolConnectAppScheme,
        kGhalBolConnectHttpsHost;

const String kGhalBolShareConnectV1 = kGhalBolConnectShareV1;
const int kInviteFormatVersionWire = kConnectInviteFormatV2;
const String kDefaultGossipTopic = "ghal-bol-chat";

/// Connect invite — format v2: `public_key_hex` only on the wire / URI.
final class GhalBolConnectInvite {
  const GhalBolConnectInvite({
    required this.topic,
    required this.publicKeyHex,
    this.peerAlias,
    this.peerId = "",
  });

  final String topic;
  final String publicKeyHex;
  final String? peerAlias;
  final String peerId;

  bool get hasPublicKey => isValidPublicKeyHex(publicKeyHex);
  bool get hasFullKeys => hasPublicKey;

  Map<String, dynamic> toWireMap() {
    final pk = resolvePublicKeyHex(storedHex: publicKeyHex) ?? "";
    if (!isValidPublicKeyHex(pk)) {
      throw StateError("invite requires a valid identity public_key_hex");
    }
    return {
      "ghalbol.share": kGhalBolShareConnectV1,
      "format_version": kInviteFormatVersionWire,
      "topic": topic,
      "public_key_hex": pk,
      if (ghalSanitizePeerAlias(peerAlias) != null)
        "peer_alias": ghalSanitizePeerAlias(peerAlias),
    };
  }

  String toInviteUri() {
    final built = GhalBolFfi.buildConnectInviteLinks(toWireMap());
    if (built != null) return built.httpsUri;
    return encodeConnectInviteUri(toWireMap());
  }

  String toInviteAppUri() {
    final built = GhalBolFfi.buildConnectInviteLinks(toWireMap());
    if (built != null) return built.appUri;
    return encodeConnectInviteAppUri(toWireMap());
  }

  static GhalBolConnectInvite? tryParseInviteUri(String input) {
    final trimmed = normalizeInviteInput(input);
    final wire =
        decodeConnectInviteUri(trimmed) ?? GhalBolFfi.parseConnectInviteWire(trimmed);
    if (wire == null) return null;
    if (!verifyInviteWire(wire)) return null;
    return _fromWire(wire);
  }

  static bool verifyInvite(GhalBolConnectInvite invite) =>
      verifyInviteWire(invite.toWireMap());

  static bool verifyInviteWire(Map<String, dynamic> wire) {
    if (GhalBolFfi.verifyGhalBolConnectInviteJson(jsonEncode(wire))) {
      return true;
    }
    return verifyConnectInviteWireMap(wire);
  }

  static String? explainInviteParseFailure(String input) {
    final trimmed = normalizeInviteInput(input);
    final wire =
        decodeConnectInviteUri(trimmed) ?? GhalBolFfi.parseConnectInviteWire(trimmed);
    if (wire == null) {
      return "Could not read that invitation link.";
    }
    if (!verifyInviteWire(wire)) {
      return "That invitation link is invalid. Ask them to share a new QR from this app.";
    }
    return null;
  }

  static GhalBolConnectInvite? _fromWire(Map<String, dynamic> map) {
    if (map["ghalbol.share"]?.toString() != kGhalBolShareConnectV1) return null;
    if (_formatVersion(map) != kInviteFormatVersionWire) return null;
    final pkRaw = map["public_key_hex"]?.toString().trim() ?? "";
    final pk = resolvePublicKeyHex(storedHex: pkRaw) ?? "";
    if (!isValidPublicKeyHex(pk)) return null;
    final pid = GhalBolFfi.peerIdFromPublicKeyHex(pk) ?? "";
    return GhalBolConnectInvite(
      topic: map["topic"]?.toString() ?? kDefaultGossipTopic,
      publicKeyHex: pk,
      peerAlias: map["peer_alias"]?.toString(),
      peerId: pid,
    );
  }

  static int? _formatVersion(Map<String, dynamic> map) {
    final raw = map["format_version"];
    if (raw is int) return raw;
    return int.tryParse(raw?.toString() ?? "");
  }
}
