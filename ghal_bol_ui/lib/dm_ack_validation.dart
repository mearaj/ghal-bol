import "public_key_hex.dart";
import "saved_contact.dart";

bool dmAckHexEq(String a, String b) => a.trim().toLowerCase() == b.trim().toLowerCase();

/// Whether an inbound ack belongs to this conversation (open chat UI guard).
bool dmAckSenderMatchesPeerKeys({
  required String senderPublicKeyHex,
  SavedContact? contact,
  String? learnedRemotePublicKeyHex,
  String? invitePublicKeyHex,
  String? ackFromPeerId,
}) {
  final sender = senderPublicKeyHex.trim();
  if (!isValidPublicKeyHex(sender)) return false;
  final fromPeer = ackFromPeerId?.trim() ?? "";
  if (contact != null &&
      fromPeer.isNotEmpty &&
      libp2pWireMatchesContactPublicKey(
        wirePeerId: fromPeer,
        contactPublicKeyHex: contact.publicKeyHex,
      )) {
    return true;
  }
  if (contact != null && contact.hasPublicKey && dmAckHexEq(sender, contact.publicKeyHex)) {
    return true;
  }
  final learned = learnedRemotePublicKeyHex?.trim() ?? "";
  if (isValidPublicKeyHex(learned) && dmAckHexEq(sender, learned)) return true;
  final inv = invitePublicKeyHex?.trim() ?? "";
  if (isValidPublicKeyHex(inv) && dmAckHexEq(sender, inv)) return true;
  return false;
}
