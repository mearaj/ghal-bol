import "invite_uri_builder.dart";
import "public_key_hex.dart";

/// Returns a Ghal Bol invitation link encoding your **public key** only.
String exportGhalBolIdentityPlaintext({
  required String? libp2pPeerId,
  required String? publicKeyHex,
  required String? appNamespace,
}) {
  final pk = publicKeyHex?.trim() ?? "";
  if (!isValidPublicKeyHex(pk)) return "";
  return buildGhalBolInviteUri(publicKeyHex: pk) ?? "";
}
