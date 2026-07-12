import "package:ghal_bol_ui/ghal_bol_connect_invite.dart";
import "package:ghal_bol_ui/identity_display_name.dart";
import "package:ghal_bol_ui/public_key_hex.dart";

/// HTTPS + `ghalbol://` pair for the same format-2 invite.
class GhalBolInviteLinks {
  const GhalBolInviteLinks({
    required this.httpsUri,
    required this.appUri,
  });

  final String httpsUri;
  final String appUri;
}

/// Format-2 invite URIs: **identity wire** and optional **peer alias** on both links.
GhalBolInviteLinks? buildGhalBolInviteLinks({
  required String publicKeyHex,
  String? peerAlias,
  String? identityWire,
}) {
  final pk = (identityWire ?? publicKeyHex).trim();
  if (!isValidPublicKeyHex(pk)) return null;
  final inv = GhalBolConnectInvite(
    topic: kDefaultGossipTopic,
    publicKeyHex: pk,
    peerAlias: ghalSanitizePeerAlias(peerAlias),
  );
  return GhalBolInviteLinks(
    httpsUri: inv.toInviteUri(),
    appUri: inv.toInviteAppUri(),
  );
}

/// `https://ghalbol.com/connect/…`
String? buildGhalBolInviteUri({
  required String publicKeyHex,
  String? peerAlias,
  String? identityWire,
}) =>
    buildGhalBolInviteLinks(
      publicKeyHex: publicKeyHex,
      peerAlias: peerAlias,
      identityWire: identityWire,
    )?.httpsUri;

/// `ghalbol://connect/…`
String? buildGhalBolInviteAppUri({
  required String publicKeyHex,
  String? peerAlias,
  String? identityWire,
}) =>
    buildGhalBolInviteLinks(
      publicKeyHex: publicKeyHex,
      peerAlias: peerAlias,
      identityWire: identityWire,
    )?.appUri;
