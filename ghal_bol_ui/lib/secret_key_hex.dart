/// secp256k1 secret key as 64 hex characters (32 bytes).
const int kSecretKeyHexLen = 64;

bool isValidSecretKeyHex(String? hex) {
  final s = hex?.trim() ?? "";
  if (s.length != kSecretKeyHexLen) return false;
  return RegExp(r"^[0-9a-fA-F]+$").hasMatch(s);
}

String normalizeSecretKeyHex(String raw) => raw.trim().toLowerCase();
