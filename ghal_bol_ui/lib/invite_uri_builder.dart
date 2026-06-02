import "package:ghal_bol_ui/ghalbol_connect_invite.dart";
import "package:ghal_bol_ui/identity_display_name.dart";
import "package:ghal_bol_ui/public_key_hex.dart";

/// Format-2 invite URI: **public key** (and optional display alias) only.
String? buildGhalBolInviteUri({
  required String publicKeyHex,
  String? peerAlias,
}) {
  final pk = publicKeyHex.trim().toLowerCase();
  if (!isValidPublicKeyHex(pk)) return null;
  return GhalBolConnectInvite(
    topic: kDefaultGossipTopic,
    publicKeyHex: pk,
    peerAlias: ghalSanitizePeerAlias(peerAlias),
  ).toInviteUri();
}
