import "package:ghal_bol_ui/ghal_bol_p2p.dart";

/// Thin wrapper for native call signaling (`p2p_call_signal`).
abstract final class GhalBolCall {
  static Future<Map<String, dynamic>> send({
    required String recipientPublicKeyHex,
    required String callId,
    required String signal,
    Map<String, dynamic> payload = const {},
    String? signalId,
  }) async {
    final cfg = <String, dynamic>{
      "recipient_public_key_hex": recipientPublicKeyHex.trim().toLowerCase(),
      "call_id": callId,
      "signal": signal,
      "payload": payload,
    };
    if (signalId != null && signalId.isNotEmpty) {
      cfg["signal_id"] = signalId;
    }
    return GhalBolP2p.callSignal(cfg);
  }
}
