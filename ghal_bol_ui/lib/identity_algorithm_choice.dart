import "package:ghal_bol_ui/ghal_bol_ffi.dart";

/// One selectable identity algorithm for first-time setup (metadata from native).
final class IdentityAlgorithmOption {
  const IdentityAlgorithmOption({
    required this.wireId,
    required this.description,
    required this.importSecretHint,
    required this.p2pReady,
    required this.isDefault,
  });

  final String wireId;
  final String description;
  final String importSecretHint;
  final bool p2pReady;
  final bool isDefault;

  String get label => wireId;
}

/// Algorithms offered at identity creation — list and copy from `ghal_bol` FFI only.
abstract final class IdentityAlgorithms {
  static const defaultWireId = "secp256k1";

  /// When native is unavailable, only the documented default is advertised.
  static const List<IdentityAlgorithmOption> _nativeUnavailableFallback = [
    IdentityAlgorithmOption(
      wireId: defaultWireId,
      description: "Default identity algorithm (requires native library to create).",
      importSecretHint: "32-byte secret as 64 hex characters",
      p2pReady: true,
      isDefault: true,
    ),
  ];

  static List<IdentityAlgorithmOption> supported() {
    final raw = GhalBolFfi.supportedIdentityAlgorithms();
    if (raw.isEmpty) return _nativeUnavailableFallback;
    return raw;
  }

  static IdentityAlgorithmOption defaultOption() {
    final list = supported();
    return list.firstWhere(
      (o) => o.isDefault,
      orElse: () => list.first,
    );
  }

  static IdentityAlgorithmOption? byWireId(String? wireId) {
    final id = wireId?.trim() ?? "";
    if (id.isEmpty) return null;
    for (final o in supported()) {
      if (o.wireId == id) return o;
    }
    return null;
  }
}
