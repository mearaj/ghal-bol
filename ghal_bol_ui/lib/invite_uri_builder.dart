import "package:ghal_bol_ui/ghalbol_connect_invite.dart";
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

/// Format-2 invite URIs: **public key** and optional **peer alias** on both links.
GhalBolInviteLinks? buildGhalBolInviteLinks({
  required String publicKeyHex,
  String? peerAlias,
}) {
  final pk = publicKeyHex.trim().toLowerCase();
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
}) =>
    buildGhalBolInviteLinks(publicKeyHex: publicKeyHex, peerAlias: peerAlias)?.httpsUri;

/// `ghalbol://connect/…`
String? buildGhalBolInviteAppUri({
  required String publicKeyHex,
  String? peerAlias,
}) =>
    buildGhalBolInviteLinks(publicKeyHex: publicKeyHex, peerAlias: peerAlias)?.appUri;
