import "invite_uri_builder.dart";
import "public_key_hex.dart";

/// Returns a Ghal Bol invitation link encoding your **identity public key** only.
String exportGhalBolIdentityPlaintext({
  required String? libp2pPeerId,
  required String? publicKeyHex,
  String? identityWire,
  required String? appNamespace,
}) {
  final wire = identityWireFromSession(
    identityWire: identityWire,
    publicKeyHex: publicKeyHex,
  );
  if (wire == null || !isValidPublicKeyHex(wire)) return "";
  return buildGhalBolInviteUri(
        publicKeyHex: publicKeyHex?.trim() ?? wire,
        identityWire: wire,
      ) ??
      "";
}
