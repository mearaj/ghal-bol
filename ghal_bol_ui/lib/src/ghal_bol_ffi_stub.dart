import "dart:convert";

import "ghal_bol_ffi_result.dart";

void _ignorePassword(String _) {}

/// Web / non‑`dart:io` — native library not loaded.
abstract final class GhalBolFfi {
  static bool get isLibraryLoaded => false;

  static String? get loadErrorText =>
      "Native library not available on this platform.";

  static void tryInitLibrary() {}

  static void configureAndroidDataDirectory(String _) {}

  static GhalBolIdentityResult createOrUnlockIdentity({
    required String appNamespace,
    required String password,
  }) {
    _ignorePassword(password);
    return GhalBolIdentityResult(
      ok: false,
      error: "$loadErrorText (namespace=$appNamespace)",
    );
  }

  static void lock() {}

  static bool get isDeleteKeystoreAvailable => false;

  static GhalBolIdentityResult deleteKeystoreVerified({
    required String appNamespace,
    required String password,
  }) {
    _ignorePassword(password);
    return GhalBolIdentityResult(
      ok: false,
      error: "$loadErrorText (namespace=$appNamespace)",
    );
  }

  static bool get isPeerDisplayAliasAvailable => false;

  static String? peerDisplayAliasGet({
    required String appNamespace,
    required String publicKeyHex,
  }) => null;

  static String? peerDisplayAliasSet({
    required String appNamespace,
    required String publicKeyHex,
    required String raw,
  }) => null;

  static bool? keystoreExists({required String appNamespace}) => null;

  static bool get isIdentityKeyManagementAvailable => false;

  static bool resetFirstTimeIdentity({required String appNamespace}) => false;

  static GhalBolIdentityResult importIdentityFromSecretHex({
    required String appNamespace,
    required String password,
    required String secretKeyHex,
  }) {
    _ignorePassword(password);
    return GhalBolIdentityResult(ok: false, error: loadErrorText);
  }

  static ({bool ok, String? secretKeyHex, String? error}) revealSecretKeyHex({
    required String appNamespace,
    required String password,
  }) => (ok: false, secretKeyHex: null, error: loadErrorText);

  static ({bool ok, String? keystoreJson, String? error}) exportKeystoreJson({
    required String appNamespace,
  }) => (ok: false, keystoreJson: null, error: loadErrorText);

  static GhalBolIdentityResult importKeystoreJson({
    required String appNamespace,
    required String password,
    required String keystoreJson,
  }) {
    _ignorePassword(password);
    return GhalBolIdentityResult(ok: false, error: loadErrorText);
  }

  static bool get isCoordAvailable => false;

  static Map<String, dynamic> coordSetBaseUrl({
    required String baseUrl,
    bool insecureTls = false,
  }) => {
    "ok": false,
    "error": loadErrorText ?? "unavailable",
  };

  static Map<String, dynamic> coordRegisterNow() => {"ok": false};

  static Map<String, dynamic> coordLookupPeer({required String publicKeyHex}) => {
    "ok": false,
    "error": loadErrorText ?? "unavailable",
  };

  static bool get isP2pAvailable => false;

  static bool get isConnectInviteCryptoAvailable => false;

  static bool verifyGhalBolConnectInviteJson(String _) => false;

  static String? peerIdFromPublicKeyHex(String _) => null;

  static String? peerIdFromSigningPublicKeyHex(String pk) => peerIdFromPublicKeyHex(pk);

  static String? publicKeyHexFromPeerId(String _) => null;

  static Map<String, dynamic> sealUtf8ToX25519Hex({
    required String recipientEncryptionPkHex,
    required String plaintext,
  }) => {
    "ok": false,
    "error": loadErrorText ?? "unavailable",
  };

  static Map<String, dynamic> openSealedCipherHex(String _) => {
    "ok": false,
    "error": loadErrorText ?? "unavailable",
  };

  static Map<String, dynamic> p2pStartJson(Map<String, dynamic> config) => {
    "ok": false,
    "error": loadErrorText ?? "unavailable",
  };

  static void p2pStop() {}

  static bool p2pIsRunning() => false;

  static Future<bool> waitP2pNodeReady({
    Duration timeout = const Duration(seconds: 8),
  }) async =>
      false;

  static void ensureChatListenerRunning() {}

  static void p2pRegisterDmPeer(String peerId, String publicKeyHex) {}

  static Map<String, dynamic> p2pSendTextDm(
    String recipientPublicKeyHex,
    String text,
  ) => {
    "ok": false,
    "error": loadErrorText ?? "unavailable",
  };

  static Map<String, dynamic> p2pCallSignal(Map<String, dynamic> config) => {
    "ok": false,
    "error": loadErrorText ?? "unavailable",
  };

  static bool get isP2pRequeueAvailable => false;

  static Map<String, dynamic> p2pRequeueOutboundDm({
    required String messageId,
    required String recipientPublicKeyHex,
    required String text,
  }) => {
    "ok": false,
    "error": loadErrorText ?? "unavailable",
  };

  static Map<String, dynamic> p2pSetForegroundPeer(String? libp2pPeerId) => {"ok": true};

  static Map<String, dynamic> p2pSetAppAckReadEnabled(bool enabled) => {"ok": true};

  static Map<String, dynamic> sealUtf8ToPublicKeyHex({
    required String recipientPublicKeyHex,
    required String plaintext,
  }) => sealUtf8ToX25519Hex(
    recipientEncryptionPkHex: recipientPublicKeyHex,
    plaintext: plaintext,
  );

  static Map<String, dynamic> p2pSendAckDm({
    required String recipientPublicKeyHex,
    required String refId,
    required String ackKind,
  }) => {
    "ok": false,
    "error": loadErrorText ?? "unavailable",
  };

  static Map<String, dynamic>? p2pPollEventMap() => null;

  static bool get isContactsStoreAvailable => false;

  static bool get isNativeServiceAvailable => false;

  static List<Map<String, dynamic>> contactsList(String _) => [];

  static Map<String, dynamic> contactsUpsert(String _, Map<String, dynamic> _) => {"ok": false};

  static bool contactsRemove(String _, Map<String, dynamic> _) => false;

  static Map<String, dynamic>? contactsFind(String _, Map<String, dynamic> _) => null;

  static bool contactsMergeDiscovered(String _, String _, String _) => false;

  static bool contactsRecordInboundPreview(String _, Map<String, dynamic> _) => false;

  static bool contactsClearUnread(String _, String _) => false;

  static Map<String, dynamic> contactsSetTrust(String _, Map<String, dynamic> _) => {"ok": false};

  static Map<String, dynamic>? coordSettingsGet({required String appNamespace}) => null;

  static String? daemonSocketPath() => null;

  static String? transcriptResolvePath(String _) => null;

  static List<Map<String, dynamic>> transcriptLoadMerged(String _, Map<String, dynamic> _) =>
      [];

  static bool transcriptSave(String _, String _, List<Map<String, dynamic>> _) => false;

  static bool transcriptAppendIfNew(String _, String _, Map<String, dynamic> _) => false;

  static bool transcriptPatchOutgoingDelivery(
    String _, {
    required String conversationKey,
    required String messageId,
    required String delivery,
  }) =>
      false;

  static bool transcriptPatchInboundReadAckSent(
    String _, {
    required String conversationKey,
    required String messageId,
  }) =>
      false;

  static String? buildConnectInviteUri(Map<String, dynamic> _) => null;

  static Map<String, dynamic>? parseConnectInviteWire(String _) => null;

  static GhalBolIdentityResult parseIdentityJson(String decoded) {
    dynamic raw;
    try {
      raw = jsonDecode(decoded);
    } catch (e) {
      return GhalBolIdentityResult(
        ok: false,
        error: "native layer invalid JSON ($e)",
      );
    }
    Map<String, dynamic>? map;
    if (raw is Map<String, dynamic>) {
      map = raw;
    } else if (raw is Map) {
      map = Map<String, dynamic>.from(raw);
    }
    if (map == null) {
      return const GhalBolIdentityResult(ok: false, error: "JSON was not an object");
    }
    final ok = map["ok"] == true;
    if (!ok) {
      return GhalBolIdentityResult(
        ok: false,
        error: map["error"]?.toString() ?? map.toString(),
      );
    }
    return GhalBolIdentityResult.fromPayload(map);
  }
}
