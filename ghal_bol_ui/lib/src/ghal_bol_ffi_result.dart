/// Result of [`GhalBolFfi.createOrUnlockIdentity`].
final class GhalBolIdentityResult {
  const GhalBolIdentityResult({
    required this.ok,
    this.error,
    this.publicKeyHex,
    this.appNamespace,
    this.libp2pPeerId,
  });

  final bool ok;
  final String? error;
  final String? publicKeyHex;
  final String? appNamespace;
  /// libp2p [`PeerId`](https://docs.libp2p.io/concepts/fundamentals/peers/#peer-id) (base58).
  final String? libp2pPeerId;

  static GhalBolIdentityResult fromPayload(Map<String, dynamic> map) {
    final ok = map["ok"] == true;
    if (!ok) {
      return GhalBolIdentityResult(
        ok: false,
        error: map["error"]?.toString() ?? map.toString(),
      );
    }
    final pk = map["public_key_hex"]?.toString();
    return GhalBolIdentityResult(
      ok: true,
      publicKeyHex: pk,
      appNamespace: map["app_namespace"]?.toString(),
      libp2pPeerId: map["libp2p_peer_id"]?.toString(),
    );
  }
}
