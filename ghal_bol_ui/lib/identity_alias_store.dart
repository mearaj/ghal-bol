import "ghal_bol_ffi.dart";
import "identity_display_name.dart";
import "public_key_hex.dart";

/// Display alias persistence through the **`ghal_bol`** native library (not Flutter prefs).
class IdentityAliasStore {
  IdentityAliasStore._();

  /// `null` = no custom alias saved (use [ghalBolIdName] default).
  static Future<String?> read({
    required String appNamespace,
    required String publicKeyHex,
  }) async {
    final pk = publicKeyHex.trim().toLowerCase();
    if (!isValidPublicKeyHex(pk)) return null;
    return GhalBolFfi.peerDisplayAliasGet(appNamespace: appNamespace, publicKeyHex: pk);
  }

  /// Empty or whitespace-only [raw] clears the stored alias (revert to default).
  static Future<void> write({
    required String appNamespace,
    required String publicKeyHex,
    required String raw,
  }) async {
    final pk = publicKeyHex.trim().toLowerCase();
    if (!isValidPublicKeyHex(pk)) return;
    GhalBolFfi.peerDisplayAliasSet(
      appNamespace: appNamespace,
      publicKeyHex: pk,
      raw: raw,
    );
  }

  /// Client-side hint only; wire values are normalized in Rust.
  static String? sanitizeForUi(String? raw) => ghalSanitizePeerAlias(raw);
}
