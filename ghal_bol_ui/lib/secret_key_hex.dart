import "package:ghal_bol_ui/ghal_bol_ffi.dart";

/// secp256k1 secret key as 64 hex characters (32 bytes) — legacy helper name.
const int kSecretKeyHexLen = 64;

String normalizeSecretKeyHex(String raw) => raw.trim().toLowerCase();

/// Legacy secp256k1-only check; import validation is native (`identity_validate_import_secret`).
bool isValidSecretKeyHex(String? hex) {
  final s = normalizeSecretKeyHex(hex ?? "");
  return GhalBolFfi.identityImportSecretValid(
    algorithm: "secp256k1",
    secretHex: s,
  );
}
