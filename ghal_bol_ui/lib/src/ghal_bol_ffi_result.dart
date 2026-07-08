/// Result of [`GhalBolFfi.createOrUnlockIdentity`].
final class GhalBolIdentityResult {
  const GhalBolIdentityResult({
    required this.ok,
    this.error,
    this.publicKeyHex,
    this.appNamespace,
    this.libp2pPeerId,
    this.identityWire,
    this.identityAlgorithm,
    this.p2pReady,
  });

  final bool ok;
  final String? error;
  final String? publicKeyHex;
  final String? appNamespace;
  /// libp2p [`PeerId`](https://docs.libp2p.io/concepts/fundamentals/peers/#peer-id) (base58).
  final String? libp2pPeerId;
  /// Full contact identity wire (`[algo:]hex` per MULTI_ALGO.md).
  final String? identityWire;
  final String? identityAlgorithm;
  /// Whether the shipping libp2p P2P stack can run with this identity.
  final bool? p2pReady;

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
      identityWire: map["identity_wire"]?.toString() ?? pk,
      identityAlgorithm: map["identity_algorithm"]?.toString(),
      p2pReady: map["p2p_ready"] == true,
    );
  }
}

/// HTTPS + `ghalbol://` invite pair from native `build_connect_invite`.
final class GhalBolNativeInviteUris {
  const GhalBolNativeInviteUris({
    required this.httpsUri,
    required this.appUri,
  });

  final String httpsUri;
  final String appUri;
}
