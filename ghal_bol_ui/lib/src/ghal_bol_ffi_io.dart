import "dart:convert";
import "dart:ffi";
import "dart:io";

import "package:ffi/ffi.dart";
import "package:ghal_bol_ui/app_log.dart";
import "package:ghal_bol_ui/native_build_hint.dart";
import "package:ghal_bol_ui/p2p_event_log.dart";
import "package:ghal_bol_ui/public_key_hex.dart";

import "ghal_bol_ffi_result.dart";

DynamicLibrary _openLib() {
  if (Platform.isAndroid) {
    return DynamicLibrary.open("libghal_bol.so");
  }

  final exeDir = File(Platform.resolvedExecutable).parent.path;

  if (Platform.isLinux) {
    final inBundle = "$exeDir/lib/libghal_bol.so";
    if (File(inBundle).existsSync()) {
      return DynamicLibrary.open(inBundle);
    }
    return DynamicLibrary.open("libghal_bol.so");
  }
  if (Platform.isMacOS) {
    for (final path in <String>[
      "$exeDir/../Frameworks/libghal_bol.dylib",
      "$exeDir/lib/libghal_bol.dylib",
    ]) {
      if (File(path).existsSync()) {
        return DynamicLibrary.open(path);
      }
    }
    return DynamicLibrary.open("libghal_bol.dylib");
  }
  if (Platform.isIOS) {
    return DynamicLibrary.open("libghal_bol.dylib");
  }
  if (Platform.isWindows) {
    final nextToExe = "$exeDir/ghal_bol.dll";
    if (File(nextToExe).existsSync()) {
      return DynamicLibrary.open(nextToExe);
    }
    return DynamicLibrary.open("ghal_bol.dll");
  }
  throw UnsupportedError("Unknown OS ${Platform.operatingSystem}");
}

typedef _NativeStringFree = Void Function(Pointer<Utf8> ptr);
typedef _NativeStringFreeDart = void Function(Pointer<Utf8> ptr);

typedef _NativeConfigureAndroid = Void Function(Pointer<Utf8> pathUtf8);

typedef _NativeTwoStringsToPtr =
    Pointer<Utf8> Function(Pointer<Utf8> a, Pointer<Utf8> b);
typedef _NativeTwoStringsToPtrDart = Pointer<Utf8> Function(Pointer<Utf8> a, Pointer<Utf8> b);
typedef _NativeLock = Void Function();
typedef _NativeLockDart = void Function();

typedef _NativePtrToPtr = Pointer<Utf8> Function(Pointer<Utf8> a);
typedef _NativePtrToPtrDart = Pointer<Utf8> Function(Pointer<Utf8> a);

typedef _NativePollPtr = Pointer<Utf8> Function();
typedef _NativePollPtrDart = Pointer<Utf8> Function();

typedef _NativeTwoPtrToPtr = Pointer<Utf8> Function(Pointer<Utf8> a, Pointer<Utf8> b);
typedef _NativeTwoPtrToPtrDart = Pointer<Utf8> Function(Pointer<Utf8> a, Pointer<Utf8> b);

typedef _NativeThreeStringsToPtr =
    Pointer<Utf8> Function(Pointer<Utf8> a, Pointer<Utf8> b, Pointer<Utf8> c);
typedef _NativeThreeStringsToPtrDart =
    Pointer<Utf8> Function(Pointer<Utf8> a, Pointer<Utf8> b, Pointer<Utf8> c);
/// FFI surface for **`libghal_bol`** / **`ghal_bol.dll`** (Rust crate **`ghal_bol`**).
abstract final class GhalBolFfi {
  static DynamicLibrary? _lib;
  static _NativeStringFreeDart? _stringFree;
  static _NativePtrToPtrDart? _p2pStart;
  static _NativeLockDart? _p2pStop;
  static _NativeTwoStringsToPtrDart? _p2pSendTextDm;
  static _NativeThreeStringsToPtrDart? _p2pRequeueOutboundDm;
  static _NativeTwoStringsToPtrDart? _p2pRegisterDmPeer;
  static _NativeThreeStringsToPtrDart? _p2pSendAckDm;
  static _NativePtrToPtrDart? _p2pCallSignal;
  static _NativePtrToPtrDart? _p2pCallMedia;
  static _NativePtrToPtrDart? _p2pCallStatus;
  static _NativePtrToPtrDart? _p2pDismissIncomingCallAlert;
  static _NativePtrToPtrDart? _p2pForceEndActiveCall;
  static _NativePtrToPtrDart? _p2pTakeIncomingCallWake;
  static _NativePtrToPtrDart? _p2pCallVideo;
  static _NativePtrToPtrDart? _p2pCallVideoFrame;
  static _NativePtrToPtrDart? _p2pCallVideoTexture;
  static _NativePtrToPtrDart? _p2pCallVideoPushCameraFrame;
  static _NativePtrToPtrDart? _callMediaKeyHex;
  static _NativePtrToPtrDart? _p2pSetForegroundPeer;
  static Pointer<Utf8> Function(int)? _p2pSetAppAckReadEnabled;
  static Pointer<Utf8> Function(int)? _p2pSetAppUiVisible;
  static _NativePollPtrDart? _p2pPollEvent;
  static _NativePollPtrDart? _p2pIsRunning;
  static _NativePtrToPtrDart? _coordSetBaseUrl;
  static _NativePtrToPtrDart? _coordLookupPeer;
  static _NativePollPtrDart? _coordRegisterNow;
  static _NativePtrToPtrDart? _verifyGhalBolConnectInvite;
  static _NativePtrToPtrDart? _peerIdFromSigningPk;
  static _NativePtrToPtrDart? _publicKeyHexFromPeerId;
  static _NativeTwoPtrToPtrDart? _sealUtf8ToX25519Hex;
  static _NativePtrToPtrDart? _openSealedCipherHex;
  static _NativePtrToPtrDart? _keystoreExistsQuery;
  static _NativeTwoStringsToPtrDart? _peerDisplayAliasGet;
  static _NativeThreeStringsToPtrDart? _peerDisplayAliasSet;
  static _NativeTwoStringsToPtrDart? _deleteKeystore;
  static _NativeThreeStringsToPtrDart? _importIdentityFromSecretHex;
  static _NativeTwoStringsToPtrDart? _revealSecretKeyHex;
  static _NativePtrToPtrDart? _exportKeystoreJson;
  static _NativeThreeStringsToPtrDart? _importKeystoreJson;
  static _NativePtrToPtrDart? _resetFirstTimeIdentity;
  static _NativePtrToPtrDart? _contactsList;
  static _NativeTwoPtrToPtrDart? _contactsUpsert;
  static _NativeTwoPtrToPtrDart? _contactsRemove;
  static _NativeTwoPtrToPtrDart? _contactsFind;
  static _NativeThreeStringsToPtrDart? _contactsMergeDiscovered;
  static _NativeTwoPtrToPtrDart? _contactsRecordPreview;
  static _NativeTwoStringsToPtrDart? _contactsClearUnread;
  static _NativeTwoPtrToPtrDart? _contactsSetTrust;
  static _NativePtrToPtrDart? _coordSettingsGet;
  static _NativePollPtrDart? _daemonSocketPath;
  static _NativePtrToPtrDart? _transcriptResolvePath;
  static _NativeTwoPtrToPtrDart? _transcriptLoadMerged;
  static _NativeTwoPtrToPtrDart? _transcriptSave;
  static _NativeTwoPtrToPtrDart? _transcriptAppend;
  static _NativeTwoPtrToPtrDart? _transcriptPatchDelivery;
  static _NativeTwoPtrToPtrDart? _transcriptPatchReadAck;
  static _NativePtrToPtrDart? _buildConnectInviteUri;
  static _NativePtrToPtrDart? _parseConnectInviteUri;
  static bool _loaded = false;
  static String? _loadError;

  static bool get isLibraryLoaded => _loaded;

  static String? get loadErrorText => _loadError;

  static void tryInitLibrary() => _ensure();

  static void _ensure() {
    if (_loaded || _lib != null) return;
    try {
      final lib = _openLib();
      _stringFree = lib.lookupFunction<_NativeStringFree, _NativeStringFreeDart>(
        "ghal_bol_ffi_string_free",
      );
      lib.lookupFunction<
        _NativeTwoStringsToPtr,
        Pointer<Utf8> Function(Pointer<Utf8>, Pointer<Utf8>)
      >("ghal_bol_ffi_create_or_unlock_identity");
      try {
        _p2pStart = lib.lookupFunction<_NativePtrToPtr, _NativePtrToPtrDart>(
          "ghal_bol_ffi_p2p_start",
        );
        _p2pStop = lib.lookupFunction<_NativeLock, _NativeLockDart>("ghal_bol_ffi_p2p_stop");
        _p2pSendTextDm = lib.lookupFunction<_NativeTwoStringsToPtr, _NativeTwoStringsToPtrDart>(
          "ghal_bol_ffi_p2p_send_text_dm",
        );
        try {
          _p2pRequeueOutboundDm = lib.lookupFunction<_NativeThreeStringsToPtr, _NativeThreeStringsToPtrDart>(
            "ghal_bol_ffi_p2p_requeue_outbound_dm",
          );
        } catch (_) {
          _p2pRequeueOutboundDm = null;
        }
        _p2pRegisterDmPeer = lib.lookupFunction<_NativeTwoStringsToPtr, _NativeTwoStringsToPtrDart>(
          "ghal_bol_ffi_p2p_register_dm_peer",
        );
        _p2pSendAckDm = lib.lookupFunction<_NativeThreeStringsToPtr, _NativeThreeStringsToPtrDart>(
          "ghal_bol_ffi_p2p_send_ack_dm",
        );
        try {
          _p2pCallSignal = lib.lookupFunction<_NativePtrToPtr, _NativePtrToPtrDart>(
            "ghal_bol_ffi_p2p_call_signal",
          );
        } catch (_) {
          _p2pCallSignal = null;
        }
        try {
          _p2pCallMedia = lib.lookupFunction<_NativePtrToPtr, _NativePtrToPtrDart>(
            "ghal_bol_ffi_p2p_call_media",
          );
        } catch (_) {
          _p2pCallMedia = null;
        }
        try {
          _p2pCallStatus = lib.lookupFunction<_NativePtrToPtr, _NativePtrToPtrDart>(
            "ghal_bol_ffi_p2p_call_status",
          );
        } catch (_) {
          _p2pCallStatus = null;
        }
        try {
          _p2pDismissIncomingCallAlert = lib.lookupFunction<
              _NativePtrToPtr, _NativePtrToPtrDart>(
            "ghal_bol_ffi_p2p_dismiss_incoming_call_alert",
          );
        } catch (_) {
          _p2pDismissIncomingCallAlert = null;
        }
        try {
          _p2pForceEndActiveCall = lib.lookupFunction<_NativePtrToPtr, _NativePtrToPtrDart>(
            "ghal_bol_ffi_p2p_force_end_active_call",
          );
        } catch (_) {
          _p2pForceEndActiveCall = null;
        }
        try {
          _p2pTakeIncomingCallWake = lib.lookupFunction<_NativePtrToPtr, _NativePtrToPtrDart>(
            "ghal_bol_ffi_p2p_take_incoming_call_wake",
          );
        } catch (_) {
          _p2pTakeIncomingCallWake = null;
        }
        try {
          _p2pCallVideo = lib.lookupFunction<_NativePtrToPtr, _NativePtrToPtrDart>(
            "ghal_bol_ffi_p2p_call_video",
          );
        } catch (_) {
          _p2pCallVideo = null;
        }
        try {
          _p2pCallVideoFrame = lib.lookupFunction<_NativePtrToPtr, _NativePtrToPtrDart>(
            "ghal_bol_ffi_p2p_call_video_frame",
          );
        } catch (_) {
          _p2pCallVideoFrame = null;
        }
        try {
          _p2pCallVideoTexture = lib.lookupFunction<_NativePtrToPtr, _NativePtrToPtrDart>(
            "ghal_bol_ffi_p2p_call_video_texture",
          );
        } catch (_) {
          _p2pCallVideoTexture = null;
        }
        try {
          _p2pCallVideoPushCameraFrame = lib.lookupFunction<_NativePtrToPtr, _NativePtrToPtrDart>(
            "ghal_bol_ffi_p2p_call_video_push_camera_frame",
          );
        } catch (_) {
          _p2pCallVideoPushCameraFrame = null;
        }
        try {
          _callMediaKeyHex = lib.lookupFunction<_NativePtrToPtr, _NativePtrToPtrDart>(
            "ghal_bol_ffi_call_media_key_hex",
          );
        } catch (_) {
          _callMediaKeyHex = null;
        }
        try {
          _p2pSetForegroundPeer = lib.lookupFunction<_NativePtrToPtr, _NativePtrToPtrDart>(
            "ghal_bol_ffi_p2p_set_foreground_peer",
          );
        } catch (_) {
          _p2pSetForegroundPeer = null;
        }
        try {
          _p2pSetAppAckReadEnabled = lib.lookupFunction<
            Pointer<Utf8> Function(Uint8),
            Pointer<Utf8> Function(int)
          >("ghal_bol_ffi_p2p_set_app_ack_read_enabled");
        } catch (_) {
          _p2pSetAppAckReadEnabled = null;
        }
        try {
          _p2pSetAppUiVisible = lib.lookupFunction<
            Pointer<Utf8> Function(Uint8),
            Pointer<Utf8> Function(int)
          >("ghal_bol_ffi_p2p_set_app_ui_visible");
        } catch (_) {
          _p2pSetAppUiVisible = null;
        }
        _p2pPollEvent = lib.lookupFunction<_NativePollPtr, _NativePollPtrDart>(
          "ghal_bol_ffi_p2p_poll_event",
        );
      } catch (_) {
        _p2pStart = null;
        _p2pStop = null;
        _p2pSendTextDm = null;
        _p2pRequeueOutboundDm = null;
        _p2pRegisterDmPeer = null;
        _p2pSendAckDm = null;
        _p2pCallSignal = null;
        _p2pCallMedia = null;
        _p2pCallVideo = null;
        _p2pCallVideoFrame = null;
        _p2pCallVideoTexture = null;
        _p2pSetForegroundPeer = null;
        _p2pSetAppAckReadEnabled = null;
        _p2pPollEvent = null;
      }
      try {
        _p2pIsRunning = lib.lookupFunction<_NativePollPtr, _NativePollPtrDart>(
          "ghal_bol_ffi_p2p_is_running",
        );
      } catch (_) {
        _p2pIsRunning = null;
      }
      try {
        _coordSetBaseUrl = lib.lookupFunction<_NativePtrToPtr, _NativePtrToPtrDart>(
          "ghal_bol_ffi_coord_set_base_url",
        );
        _coordLookupPeer = lib.lookupFunction<_NativePtrToPtr, _NativePtrToPtrDart>(
          "ghal_bol_ffi_coord_lookup_peer",
        );
        _coordRegisterNow = lib.lookupFunction<_NativePollPtr, _NativePollPtrDart>(
          "ghal_bol_ffi_coord_register_now",
        );
      } catch (_) {
        _coordSetBaseUrl = null;
        _coordLookupPeer = null;
        _coordRegisterNow = null;
      }
      try {
        _verifyGhalBolConnectInvite = lib.lookupFunction<_NativePtrToPtr, _NativePtrToPtrDart>(
          "ghal_bol_ffi_verify_ghal_bol_connect_invite",
        );
        _sealUtf8ToX25519Hex = lib.lookupFunction<_NativeTwoPtrToPtr, _NativeTwoPtrToPtrDart>(
          "ghal_bol_ffi_seal_utf8_to_public_key_hex",
        );
        _openSealedCipherHex = lib.lookupFunction<_NativePtrToPtr, _NativePtrToPtrDart>(
          "ghal_bol_ffi_open_sealed_cipher_hex",
        );
      } catch (_) {
        _verifyGhalBolConnectInvite = null;
        _sealUtf8ToX25519Hex = null;
        _openSealedCipherHex = null;
      }
      try {
        _peerIdFromSigningPk = lib.lookupFunction<_NativePtrToPtr, _NativePtrToPtrDart>(
          "ghal_bol_ffi_peer_id_from_public_key_hex",
        );
        _publicKeyHexFromPeerId = lib.lookupFunction<_NativePtrToPtr, _NativePtrToPtrDart>(
          "ghal_bol_ffi_public_key_hex_from_peer_id",
        );
      } catch (_) {
        _peerIdFromSigningPk = null;
        _publicKeyHexFromPeerId = null;
      }
      try {
        _keystoreExistsQuery = lib.lookupFunction<_NativePtrToPtr, _NativePtrToPtrDart>(
          "ghal_bol_ffi_keystore_exists",
        );
      } catch (_) {
        _keystoreExistsQuery = null;
      }
      try {
        _peerDisplayAliasGet = lib.lookupFunction<_NativeTwoStringsToPtr, _NativeTwoStringsToPtrDart>(
          "ghal_bol_ffi_peer_display_alias_get",
        );
        _peerDisplayAliasSet = lib.lookupFunction<_NativeThreeStringsToPtr, _NativeThreeStringsToPtrDart>(
          "ghal_bol_ffi_peer_display_alias_set",
        );
      } catch (_) {
        _peerDisplayAliasGet = null;
        _peerDisplayAliasSet = null;
      }
      try {
        _deleteKeystore = lib.lookupFunction<_NativeTwoStringsToPtr, _NativeTwoStringsToPtrDart>(
          "ghal_bol_ffi_delete_keystore",
        );
      } catch (_) {
        _deleteKeystore = null;
      }
      try {
        _importIdentityFromSecretHex =
            lib.lookupFunction<_NativeThreeStringsToPtr, _NativeThreeStringsToPtrDart>(
              "ghal_bol_ffi_import_identity_from_secret_hex",
            );
        _revealSecretKeyHex = lib.lookupFunction<_NativeTwoStringsToPtr, _NativeTwoStringsToPtrDart>(
          "ghal_bol_ffi_reveal_secret_key_hex",
        );
        _exportKeystoreJson = lib.lookupFunction<_NativePtrToPtr, _NativePtrToPtrDart>(
          "ghal_bol_ffi_export_keystore_json",
        );
        _importKeystoreJson = lib.lookupFunction<_NativeThreeStringsToPtr, _NativeThreeStringsToPtrDart>(
          "ghal_bol_ffi_import_keystore_json",
        );
        _resetFirstTimeIdentity = lib.lookupFunction<_NativePtrToPtr, _NativePtrToPtrDart>(
          "ghal_bol_ffi_reset_first_time_identity",
        );
      } catch (_) {
        _importIdentityFromSecretHex = null;
        _revealSecretKeyHex = null;
        _exportKeystoreJson = null;
        _importKeystoreJson = null;
        _resetFirstTimeIdentity = null;
      }
      _loadServiceSymbols(lib);
      _lib = lib;
      _loaded = true;
      _loadError = null;
      AppLog.instance.i("FFI", "libghal_bol loaded (${Platform.operatingSystem})");
    } catch (e, st) {
      // `flutter test` on CI has no bundled `libghal_bol.so` — expected, not an error.
      final inFlutterTest = Platform.environment["FLUTTER_TEST"] == "true";
      if (!inFlutterTest) {
        AppLog.instance.e("FFI", "libghal_bol load failed", e, st);
      }
      _lib = null;
      _stringFree = null;
      _p2pStart = null;
      _p2pStop = null;
      _p2pSendTextDm = null;
      _p2pRequeueOutboundDm = null;
      _p2pRegisterDmPeer = null;
      _p2pSendAckDm = null;
      _p2pCallSignal = null;
      _p2pCallMedia = null;
      _p2pCallVideo = null;
      _p2pCallVideoFrame = null;
      _p2pCallVideoTexture = null;
      _p2pPollEvent = null;
      _p2pIsRunning = null;
      _verifyGhalBolConnectInvite = null;
      _peerIdFromSigningPk = null;
      _publicKeyHexFromPeerId = null;
      _sealUtf8ToX25519Hex = null;
      _openSealedCipherHex = null;
      _keystoreExistsQuery = null;
      _peerDisplayAliasGet = null;
      _peerDisplayAliasSet = null;
      _deleteKeystore = null;
      _importIdentityFromSecretHex = null;
      _revealSecretKeyHex = null;
      _exportKeystoreJson = null;
      _importKeystoreJson = null;
      _resetFirstTimeIdentity = null;
      _clearServiceSymbols();
      assert(() {
        if (!inFlutterTest) {
          // ignore: avoid_print
          print("GhalBolFfi failed to load: $e\n$st");
        }
        return true;
      }());
      var err = "$e";
      if (err.contains("libghal_bol") || err.contains("dlopen")) {
        err += "\n\n${NativeBuildHint.rebuildInstructions}";
      }
      _loadError = err;
    }
  }

  /// Call with Android app internal files path (matches Rust override data dir).
  static void configureAndroidDataDirectory(String path) {
    if (!Platform.isAndroid) return;
    AppLog.instance.i("FFI", "configure_android_data_directory path=$path");
    _ensure();
    final lib = _lib;
    if (lib == null) return;
    try {
      final sym = lib.lookup<
        NativeFunction<_NativeConfigureAndroid>
      >("ghal_bol_ffi_configure_android_data_directory");
      final f = sym.asFunction<void Function(Pointer<Utf8>)>();
      final p = path.toNativeUtf8();
      try {
        f(p);
      } finally {
        calloc.free(p);
      }
    } catch (_) {
      // Older .so missing symbol — ignore.
    }
  }

  /// Whether `keystore_v1.json` exists for [appNamespace] (same paths as [createOrUnlockIdentity]).
  /// Returns `null` if the native library is unavailable or lacks `ghal_bol_ffi_keystore_exists`.
  static bool? keystoreExists({required String appNamespace}) {
    _ensure();
    final lib = _lib;
    final q = _keystoreExistsQuery;
    final free = _stringFree;
    if (lib == null || q == null || free == null) return null;
    final a = appNamespace.toNativeUtf8();
    try {
      final outPtr = q(a);
      if (outPtr == nullptr || outPtr.address == 0) return null;
      late final String decoded;
      try {
        decoded = outPtr.toDartString();
      } finally {
        free(outPtr);
      }
      dynamic raw;
      try {
        raw = jsonDecode(decoded);
      } catch (_) {
        return null;
      }
      if (raw is! Map) return null;
      final map = Map<String, dynamic>.from(raw);
      if (map["ok"] != true) return null;
      final v = map["keystore_exists"];
      if (v is bool) return v;
      return null;
    } finally {
      calloc.free(a);
    }
  }

  static GhalBolIdentityResult createOrUnlockIdentity({
    required String appNamespace,
    required String password,
  }) {
    AppLog.instance.i("Identity", "create_or_unlock start namespace=$appNamespace");
    _ensure();
    final lib = _lib;
    if (lib == null) {
      AppLog.instance.e("Identity", "create_or_unlock: native library unavailable");
      return GhalBolIdentityResult(
        ok: false,
        error: _loadError ?? NativeBuildHint.libraryUnavailable,
      );
    }

    final create = lib.lookupFunction<
      _NativeTwoStringsToPtr,
      Pointer<Utf8> Function(Pointer<Utf8>, Pointer<Utf8>)
    >("ghal_bol_ffi_create_or_unlock_identity");

    final a = appNamespace.toNativeUtf8();
    final b = password.toNativeUtf8();
    try {
      final outPtr = create(a, b);
      final r = _parseRustJsonPayload(outPtr);
      _logIdentityResult("create_or_unlock", r);
      return r;
    } finally {
      calloc.free(a);
      calloc.free(b);
    }
  }

  static void _logIdentityResult(String action, GhalBolIdentityResult r) {
    if (r.ok) {
      AppLog.instance.json("Identity", "$action ok", {
        "public_key_hex": r.publicKeyHex,
        "app_namespace": r.appNamespace,
      });
      final pid = r.libp2pPeerId?.trim() ?? "";
      if (pid.isNotEmpty && AppLog.logNativeDebug) {
        AppLog.instance.d("Identity", "derived libp2p_peer_id=$pid (not on invite QR)");
      }
    } else {
      AppLog.instance.w("Identity", "$action failed: ${r.error}");
    }
  }

  /// Clears decrypted keys from RAM. Does not stop P2P — call [p2pStop] first when signing out.
  static void lock() {
    AppLog.instance.i("Identity", "lock");
    _ensure();
    final lib = _lib;
    if (lib == null) return;
    try {
      final f =
          lib
              .lookupFunction<_NativeLock, void Function()>("ghal_bol_ffi_lock");
      f();
    } catch (_) {}
  }

  /// Native exposes [`ghal_bol_ffi_delete_keystore`] (rebuild `libghal_bol` if false).
  static bool get isDeleteKeystoreAvailable => _loaded && _deleteKeystore != null && _stringFree != null;

  /// Native identity import/export/reveal (`ghal_bol_ffi_*`); rebuild lib if false.
  static bool get isIdentityKeyManagementAvailable =>
      _loaded &&
      _importIdentityFromSecretHex != null &&
      _revealSecretKeyHex != null &&
      _exportKeystoreJson != null &&
      _importKeystoreJson != null &&
      _stringFree != null;

  /// After a failed first-time create/import, remove any partial keystore (no password).
  static bool resetFirstTimeIdentity({required String appNamespace}) {
    _ensure();
    final f = _resetFirstTimeIdentity;
    final free = _stringFree;
    if (f == null || free == null) return false;
    final a = appNamespace.toNativeUtf8();
    try {
      final map = _parseRustJsonMap(f(a));
      return map["ok"] == true;
    } finally {
      calloc.free(a);
    }
  }

  /// First-time setup: import 64-hex secp256k1 secret with app password.
  static GhalBolIdentityResult importIdentityFromSecretHex({
    required String appNamespace,
    required String password,
    required String secretKeyHex,
  }) {
    AppLog.instance.w("Identity", "import_secret_hex start");
    _ensure();
    final f = _importIdentityFromSecretHex;
    if (f == null) {
      return const GhalBolIdentityResult(
        ok: false,
        error: "Import identity is not available in this native build. Re-sync libghal_bol.",
      );
    }
    final a = appNamespace.toNativeUtf8();
    final b = password.toNativeUtf8();
    final c = secretKeyHex.trim().toNativeUtf8();
    try {
      return _parseRustJsonPayload(f(a, b, c));
    } finally {
      calloc.free(a);
      calloc.free(b);
      calloc.free(c);
    }
  }

  /// Verify app password and return 64-hex secret (sensitive).
  static ({bool ok, String? secretKeyHex, String? error}) revealSecretKeyHex({
    required String appNamespace,
    required String password,
  }) {
    _ensure();
    final f = _revealSecretKeyHex;
    if (f == null) {
      return (ok: false, secretKeyHex: null, error: "Reveal key is not available in this native build.");
    }
    final a = appNamespace.toNativeUtf8();
    final b = password.toNativeUtf8();
    try {
      final map = _parseRustJsonMap(f(a, b));
      if (map["ok"] != true) {
        return (ok: false, secretKeyHex: null, error: map["error"]?.toString() ?? "reveal failed");
      }
      final hex = map["secret_key_hex"]?.toString().trim() ?? "";
      if (hex.length != 64) {
        return (ok: false, secretKeyHex: null, error: "invalid secret from native");
      }
      return (ok: true, secretKeyHex: hex.toLowerCase(), error: null);
    } finally {
      calloc.free(a);
      calloc.free(b);
    }
  }

  /// Encrypted keystore JSON (safe to copy; password required to decrypt).
  static ({bool ok, String? keystoreJson, String? error}) exportKeystoreJson({
    required String appNamespace,
  }) {
    _ensure();
    final f = _exportKeystoreJson;
    if (f == null) {
      return (ok: false, keystoreJson: null, error: "Export is not available in this native build.");
    }
    final a = appNamespace.toNativeUtf8();
    try {
      final map = _parseRustJsonMap(f(a));
      if (map["ok"] != true) {
        return (ok: false, keystoreJson: null, error: map["error"]?.toString() ?? "export failed");
      }
      final json = map["keystore_json"]?.toString() ?? "";
      if (json.isEmpty) {
        return (ok: false, keystoreJson: null, error: "empty keystore export");
      }
      return (ok: true, keystoreJson: json, error: null);
    } finally {
      calloc.free(a);
    }
  }

  /// Restore encrypted keystore when none exists on device.
  static GhalBolIdentityResult importKeystoreJson({
    required String appNamespace,
    required String password,
    required String keystoreJson,
  }) {
    AppLog.instance.w("Identity", "import_keystore_json start");
    _ensure();
    final f = _importKeystoreJson;
    if (f == null) {
      return const GhalBolIdentityResult(
        ok: false,
        error: "Import keystore is not available in this native build.",
      );
    }
    final a = appNamespace.toNativeUtf8();
    final b = password.toNativeUtf8();
    final c = keystoreJson.toNativeUtf8();
    try {
      return _parseRustJsonPayload(f(a, b, c));
    } finally {
      calloc.free(a);
      calloc.free(b);
      calloc.free(c);
    }
  }

  static Map<String, dynamic> _parseRustJsonMap(Pointer<Utf8> outPtr) {
    final free = _stringFree;
    if (free == null) return {"ok": false, "error": "ffi unavailable"};
    if (outPtr == nullptr || outPtr.address == 0) {
      return {"ok": false, "error": "null response"};
    }
    try {
      final raw = jsonDecode(outPtr.toDartString());
      if (raw is Map) return Map<String, dynamic>.from(raw);
      return {"ok": false, "error": "invalid json"};
    } catch (e) {
      return {"ok": false, "error": "$e"};
    } finally {
      free(outPtr);
    }
  }

  /// Deletes persisted keystore + preferences after verifying [password] (same as unlock).
  static GhalBolIdentityResult deleteKeystoreVerified({
    required String appNamespace,
    required String password,
  }) {
    AppLog.instance.w("Identity", "delete_keystore start namespace=$appNamespace");
    _ensure();
    final del = _deleteKeystore;
    final free = _stringFree;
    if (del == null || free == null) {
      return GhalBolIdentityResult(
        ok: false,
        error:
            "Delete identity is not available in this native build. ${NativeBuildHint.rebuildInstructions}",
      );
    }
    final a = appNamespace.toNativeUtf8();
    final b = password.toNativeUtf8();
    try {
      final outPtr = del(a, b);
      final r = _parseRustJsonPayload(outPtr);
      if (r.ok) {
        AppLog.instance.i("Identity", "delete_keystore ok");
      } else {
        AppLog.instance.w("Identity", "delete_keystore failed: ${r.error}");
      }
      return r;
    } finally {
      calloc.free(a);
      calloc.free(b);
    }
  }

  /// Whether native `ghal_bol` exposes display-alias persistence (rebuild if false).
  static bool get isPeerDisplayAliasAvailable =>
      _loaded && _peerDisplayAliasGet != null && _peerDisplayAliasSet != null && _stringFree != null;

  /// Read persisted display alias via **`ghal_bol`** (requires unlocked session).
  /// Returns `null` if unset, on error, or if symbols are missing.
  static String? peerDisplayAliasGet({
    required String appNamespace,
    required String publicKeyHex,
  }) {
    _ensure();
    final f = _peerDisplayAliasGet;
    final free = _stringFree;
    if (f == null || free == null) return null;
    final a = appNamespace.toNativeUtf8();
    final b = publicKeyHex.trim().toNativeUtf8();
    try {
      final out = f(a, b);
      final r = _parseSmallJson(out, free);
      if (r["ok"] != true) return null;
      final al = r["alias"];
      if (al == null) return null;
      final s = al.toString().trim();
      return s.isEmpty ? null : s;
    } finally {
      calloc.free(a);
      calloc.free(b);
    }
  }

  /// Write display alias via **`ghal_bol`** (empty [raw] clears). Returns stored value or `null`.
  static String? peerDisplayAliasSet({
    required String appNamespace,
    required String publicKeyHex,
    required String raw,
  }) {
    _ensure();
    final f = _peerDisplayAliasSet;
    final free = _stringFree;
    if (f == null || free == null) return null;
    final a = appNamespace.toNativeUtf8();
    final b = publicKeyHex.trim().toNativeUtf8();
    final c = raw.toNativeUtf8();
    try {
      final out = f(a, b, c);
      final r = _parseSmallJson(out, free);
      if (r["ok"] != true) return null;
      final al = r["alias"];
      if (al == null) return null;
      final s = al.toString().trim();
      return s.isEmpty ? null : s;
    } finally {
      calloc.free(a);
      calloc.free(b);
      calloc.free(c);
    }
  }

  static GhalBolIdentityResult _parseRustJsonPayload(Pointer<Utf8> outPtr) {
    if (outPtr == nullptr || outPtr.address == 0) {
      return const GhalBolIdentityResult(
        ok: false,
        error: "native layer returned null payload",
      );
    }

    late final String decoded;
    try {
      decoded = outPtr.toDartString();
    } finally {
      _stringFree?.call(outPtr);
    }

    return parseIdentityJson(decoded);
  }

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
      return const GhalBolIdentityResult(
        ok: false,
        error: "JSON was not an object",
      );
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

  static bool get isCoordAvailable =>
      _loaded && _coordSetBaseUrl != null && _coordLookupPeer != null;

  static Map<String, dynamic> coordSetBaseUrls({
    required List<String> baseUrls,
    bool insecureTls = false,
  }) {
    _ensure();
    final f = _coordSetBaseUrl;
    final free = _stringFree;
    if (f == null || free == null) {
      return {"ok": false, "error": "coord not available in this build"};
    }
    final j = jsonEncode({
      "base_urls": baseUrls,
      "insecure_tls": insecureTls,
    });
    final p = j.toNativeUtf8();
    try {
      return _parseSmallJson(f(p), free);
    } finally {
      calloc.free(p);
    }
  }

  static Map<String, dynamic> coordLookupPeer({required String publicKeyHex}) {
    _ensure();
    final f = _coordLookupPeer;
    final free = _stringFree;
    if (f == null || free == null) {
      return {"ok": false, "error": "coord not available in this build"};
    }
    final j = jsonEncode({"public_key_hex": publicKeyHex});
    final p = j.toNativeUtf8();
    try {
      return _parseSmallJson(f(p), free);
    } finally {
      calloc.free(p);
    }
  }

  static Map<String, dynamic> coordRegisterNow() {
    _ensure();
    final f = _coordRegisterNow;
    final free = _stringFree;
    if (f == null || free == null) {
      return {"ok": false, "error": "coord register not available in this build"};
    }
    return _parseSmallJson(f(), free);
  }

  /// Whether gossip P2P symbols were found in the native library.
  static bool get isP2pAvailable =>
      _loaded &&
      _p2pStart != null &&
      _p2pStop != null &&
      _p2pSendTextDm != null &&
      _p2pSendAckDm != null &&
      _p2pPollEvent != null;

  /// Restores unacked outbound rows into the native outbox (same `message_id`).
  static bool get isP2pRequeueAvailable => isP2pAvailable && _p2pRequeueOutboundDm != null;

  /// Invite verification + X25519 sealed payloads (rebuild native if missing). Signing FFI is optional.
  static bool get isConnectInviteCryptoAvailable =>
      _loaded &&
      _verifyGhalBolConnectInvite != null &&
      _sealUtf8ToX25519Hex != null &&
      _openSealedCipherHex != null;

  /// Start libp2p stream DM in a background thread. `config` is JSON, e.g.
  /// `{ "bootstrap_peers": [], "dm_peers": [{ "public_key_hex": "<66-hex>" }] }`.
  static Map<String, dynamic> p2pStartJson(Map<String, dynamic> config) {
    final dm = config["dm_peers"];
    AppLog.instance.json("P2P", "p2p_start", {
      "app_namespace": config["app_namespace"],
      "dm_peer_count": dm is List ? dm.length : 0,
      if (config["transcript_path"] != null)
        "transcript_path": config["transcript_path"],
    });
    _ensure();
    final start = _p2pStart;
    final free = _stringFree;
    if (start == null || free == null) {
      final err = {"ok": false, "error": "P2P not available in this build"};
      AppLog.instance.w("P2P", "p2p_start unavailable");
      return err;
    }
    final j = jsonEncode(config);
    final p = j.toNativeUtf8();
    try {
      final out = start(p);
      final r = _parseSmallJson(out, free);
      AppLog.instance.json("P2P", "p2p_start result", r);
      return r;
    } finally {
      calloc.free(p);
    }
  }

  static void p2pStop() {
    AppLog.instance.i("P2P", "p2p_stop");
    final stop = _p2pStop;
    if (stop == null) return;
    try {
      stop();
    } catch (_) {}
  }

  /// Whether the stream DM listener thread is currently running.
  static bool p2pIsRunning() {
    _ensure();
    final f = _p2pIsRunning;
    final free = _stringFree;
    if (f == null || free == null) return false;
    final out = f();
    if (out == nullptr || out.address == 0) return false;
    try {
      final r = _parseSmallJson(out, free);
      return r["ok"] == true && r["running"] == true;
    } catch (_) {
      return false;
    }
  }

  /// Waits until the native node emits `node_ready`, or fails / times out.
  static Future<bool> waitP2pNodeReady({
    Duration timeout = const Duration(seconds: 8),
  }) async {
    final deadline = DateTime.now().add(timeout);
    while (DateTime.now().isBefore(deadline)) {
      var drained = false;
      while (true) {
        final ev = p2pPollEventMap();
        if (ev == null) break;
        drained = true;
        logP2pEvent(ev);
        final kind = ev["kind"]?.toString();
        if (kind == "node_ready") return true;
        if (kind == "node_stopped") {
          final err = ev["error"]?.toString();
          if (err != null && err.isNotEmpty) {
            AppLog.instance.e("P2P", "node_stopped: $err");
          } else {
            AppLog.instance.e("P2P", "node_stopped (no error detail)");
          }
          return false;
        }
      }
      if (!drained && !p2pIsRunning()) return false;
      await Future<void>.delayed(const Duration(milliseconds: 50));
    }
    return p2pIsRunning();
  }

  /// Starts the chat listener if P2P is available and not already running (no bootstrap peers).
  static void ensureChatListenerRunning() {
    if (!isP2pAvailable) return;
    if (p2pIsRunning()) return;
    p2pStartJson({
      "bootstrap_peers": <String>[],
      "dm_peers": <Map<String, dynamic>>[],
    });
  }

  static void p2pRegisterDmPeer(String peerId, String publicKeyHex) {
    _ensure();
    final reg = _p2pRegisterDmPeer;
    final free = _stringFree;
    if (reg == null || free == null) return;
    final a = peerId.toNativeUtf8();
    final b = publicKeyHex.toNativeUtf8();
    try {
      final out = reg(a, b);
      final r = _parseSmallJson(out, free);
      if (r["ok"] != true) {
        AppLog.instance.w("P2P", "register_dm_peer failed: ${r["error"]}");
      }
    } finally {
      calloc.free(a);
      calloc.free(b);
    }
  }

  static Map<String, dynamic> p2pSendTextDm(String recipientPublicKeyHex, String text) {
    if (AppLog.logNativeDebug) {
      AppLog.instance.d("P2P", "send_text_dm len=${text.length}");
    }
    _ensure();
    final send = _p2pSendTextDm;
    final free = _stringFree;
    if (send == null || free == null) {
      return {"ok": false, "error": "P2P not available"};
    }
    final a = recipientPublicKeyHex.toNativeUtf8();
    final b = text.toNativeUtf8();
    try {
      final out = send(a, b);
      final r = _parseSmallJson(out, free);
      if (AppLog.logNativeDebug || r["ok"] != true) {
        AppLog.instance.json("P2P", "send_text_dm result", r);
      }
      return r;
    } finally {
      calloc.free(a);
      calloc.free(b);
    }
  }

  /// Re-queue a prior outbound text (history resync after restart).
  static Map<String, dynamic> p2pRequeueOutboundDm({
    required String messageId,
    required String recipientPublicKeyHex,
    required String text,
  }) {
    _ensure();
    final requeue = _p2pRequeueOutboundDm;
    final free = _stringFree;
    if (requeue == null || free == null) {
      return {"ok": false, "error": "P2P requeue not available (rebuild native lib)"};
    }
    final a = messageId.toNativeUtf8();
    final b = recipientPublicKeyHex.toNativeUtf8();
    final c = text.toNativeUtf8();
    try {
      final out = requeue(a, b, c);
      return _parseSmallJson(out, free);
    } finally {
      calloc.free(a);
      calloc.free(b);
      calloc.free(c);
    }
  }

  static Map<String, dynamic> p2pSetAppAckReadEnabled(bool enabled) {
    _ensure();
    final set = _p2pSetAppAckReadEnabled;
    final free = _stringFree;
    if (set == null || free == null) {
      return {"ok": false, "error": "P2P app ack read gate not available (rebuild native lib)"};
    }
    final out = set(enabled ? 1 : 0);
    return _parseSmallJson(out, free);
  }

  static Map<String, dynamic> p2pSetAppUiVisible(bool visible) {
    _ensure();
    final set = _p2pSetAppUiVisible;
    final free = _stringFree;
    if (set == null || free == null) {
      return {"ok": true, "visible": visible};
    }
    final out = set(visible ? 1 : 0);
    return _parseSmallJson(out, free);
  }

  static Map<String, dynamic> p2pSetForegroundPeer(String? libp2pPeerId) {
    _ensure();
    final set = _p2pSetForegroundPeer;
    final free = _stringFree;
    if (set == null || free == null) {
      return {"ok": false, "error": "P2P foreground peer not available (rebuild native lib)"};
    }
    final pid = libp2pPeerId?.trim() ?? "";
    if (pid.isEmpty) {
      final out = set(nullptr);
      return _parseSmallJson(out, free);
    }
    final p = pid.toNativeUtf8();
    try {
      final out = set(p);
      return _parseSmallJson(out, free);
    } finally {
      calloc.free(p);
    }
  }

  static Map<String, dynamic> p2pCallSignal(Map<String, dynamic> config) {
    _ensure();
    final send = _p2pCallSignal;
    final free = _stringFree;
    if (send == null || free == null) {
      return {"ok": false, "error": "call signaling not available in this build"};
    }
    final j = jsonEncode(config);
    final p = j.toNativeUtf8();
    try {
      return _parseSmallJson(send(p), free);
    } finally {
      calloc.free(p);
    }
  }

  static Map<String, dynamic> p2pCallMedia(Map<String, dynamic> config) {
    _ensure();
    final send = _p2pCallMedia;
    final free = _stringFree;
    if (send == null || free == null) {
      return {"ok": false, "error": "native call media not available in this build"};
    }
    final j = jsonEncode(config);
    final p = j.toNativeUtf8();
    try {
      return _parseSmallJson(send(p), free);
    } finally {
      calloc.free(p);
    }
  }

  static Map<String, dynamic> p2pCallStatus(Map<String, dynamic> config) {
    _ensure();
    final send = _p2pCallStatus;
    final free = _stringFree;
    if (send == null || free == null) {
      return {"ok": true, "active": false};
    }
    final j = jsonEncode(config);
    final p = j.toNativeUtf8();
    try {
      return _parseSmallJson(send(p), free);
    } finally {
      calloc.free(p);
    }
  }

  static Future<void> p2pDismissIncomingCallAlert() async {
    _ensure();
    final send = _p2pDismissIncomingCallAlert;
    final free = _stringFree;
    if (send == null || free == null) return;
    final p = "{}".toNativeUtf8();
    try {
      _parseSmallJson(send(p), free);
    } finally {
      calloc.free(p);
    }
  }

  static Map<String, dynamic> p2pForceEndActiveCall(Map<String, dynamic> config) {
    _ensure();
    final send = _p2pForceEndActiveCall;
    final free = _stringFree;
    if (send == null || free == null) {
      return {"ok": true, "ended": false};
    }
    final j = jsonEncode(config);
    final p = j.toNativeUtf8();
    try {
      return _parseSmallJson(send(p), free);
    } finally {
      calloc.free(p);
    }
  }

  static Map<String, dynamic> p2pTakeIncomingCallWake() {
    _ensure();
    final send = _p2pTakeIncomingCallWake;
    final free = _stringFree;
    if (send == null || free == null) {
      return {"ok": true, "wake": false};
    }
    final p = "{}".toNativeUtf8();
    try {
      return _parseSmallJson(send(p), free);
    } finally {
      calloc.free(p);
    }
  }

  static Map<String, dynamic> p2pCallVideo(Map<String, dynamic> config) {
    _ensure();
    final send = _p2pCallVideo;
    final free = _stringFree;
    if (send == null || free == null) {
      return {"ok": false, "error": "native call video not available in this build"};
    }
    final j = jsonEncode(config);
    final p = j.toNativeUtf8();
    try {
      return _parseSmallJson(send(p), free);
    } finally {
      calloc.free(p);
    }
  }

  static Map<String, dynamic> p2pCallVideoFrame(Map<String, dynamic> config) {
    _ensure();
    final send = _p2pCallVideoFrame;
    final free = _stringFree;
    if (send == null || free == null) {
      return {"ok": false, "error": "native call video frame pull not available in this build"};
    }
    final j = jsonEncode(config);
    final p = j.toNativeUtf8();
    try {
      return _parseSmallJson(send(p), free);
    } finally {
      calloc.free(p);
    }
  }

  static Map<String, dynamic> p2pCallVideoTexture(Map<String, dynamic> config) {
    _ensure();
    final send = _p2pCallVideoTexture;
    final free = _stringFree;
    if (send == null || free == null) {
      return {"ok": false, "error": "native call video texture not available in this build"};
    }
    final j = jsonEncode(config);
    final p = j.toNativeUtf8();
    try {
      return _parseSmallJson(send(p), free);
    } finally {
      calloc.free(p);
    }
  }

  static Map<String, dynamic> p2pCallVideoPushCameraFrame(Map<String, dynamic> config) {
    _ensure();
    final send = _p2pCallVideoPushCameraFrame;
    final free = _stringFree;
    if (send == null || free == null) {
      return {"ok": false, "error": "native call video push frame not available in this build"};
    }
    final j = jsonEncode(config);
    final p = j.toNativeUtf8();
    try {
      return _parseSmallJson(send(p), free);
    } finally {
      calloc.free(p);
    }
  }

  static Map<String, dynamic> callMediaKeyHex({
    required String callId,
    required String peerPublicKeyHex,
  }) {
    _ensure();
    final derive = _callMediaKeyHex;
    final free = _stringFree;
    if (derive == null || free == null) {
      return {
        "ok": false,
        "error": "call media key not available (rebuild native lib)",
      };
    }
    final j = jsonEncode({
      "call_id": callId,
      "peer_public_key_hex": peerPublicKeyHex,
    });
    final p = j.toNativeUtf8();
    try {
      return _parseSmallJson(derive(p), free);
    } finally {
      calloc.free(p);
    }
  }

  static Map<String, dynamic> p2pSendAckDm({
    required String recipientPublicKeyHex,
    required String refId,
    required String ackKind,
  }) {
    _ensure();
    final send = _p2pSendAckDm;
    final free = _stringFree;
    if (send == null || free == null) {
      return {"ok": false, "error": "P2P not available"};
    }
    final a = recipientPublicKeyHex.toNativeUtf8();
    final b = refId.toNativeUtf8();
    final c = ackKind.toNativeUtf8();
    try {
      final out = send(a, b, c);
      return _parseSmallJson(out, free);
    } finally {
      calloc.free(a);
      calloc.free(b);
      calloc.free(c);
    }
  }

  static bool verifyGhalBolConnectInviteJson(String inviteJson) {
    AppLog.instance.d("Invite", "verify invite json len=${inviteJson.length}");
    _ensure();
    final verify = _verifyGhalBolConnectInvite;
    final free = _stringFree;
    if (verify == null || free == null) return false;
    final p = inviteJson.toNativeUtf8();
    try {
      final out = verify(p);
      final r = _parseSmallJson(out, free);
      final ok = r["ok"] == true;
      AppLog.instance.i("Invite", "verify invite ${ok ? "ok" : "rejected"}");
      return ok;
    } finally {
      calloc.free(p);
    }
  }

  /// Libp2p PeerId string from 66-hex compressed secp256k1 public key.
  static String? peerIdFromPublicKeyHex(String publicKeyHex) {
    _ensure();
    final f = _peerIdFromSigningPk;
    final free = _stringFree;
    if (f == null || free == null) return null;
    final p = publicKeyHex.trim().toNativeUtf8();
    try {
      final out = f(p);
      if (out == nullptr || out.address == 0) return null;
      try {
        final s = out.toDartString();
        final raw = jsonDecode(s);
        if (raw is Map && raw["ok"] == true) {
          final id = raw["peer_id"]?.toString();
          if (id != null && id.isNotEmpty) return id;
        }
        return null;
      } catch (_) {
        return null;
      } finally {
        free(out);
      }
    } finally {
      calloc.free(p);
    }
  }

  static String? peerIdFromSigningPublicKeyHex(String signingPublicKeyHex) =>
      peerIdFromPublicKeyHex(signingPublicKeyHex);

  /// 66-hex secp256k1 public key embedded in a libp2p identity PeerId (format-3 QR).
  static String? publicKeyHexFromPeerId(String peerId) {
    _ensure();
    final f = _publicKeyHexFromPeerId;
    final free = _stringFree;
    if (f == null || free == null) return null;
    final p = peerId.trim().toNativeUtf8();
    try {
      final out = f(p);
      if (out == nullptr || out.address == 0) return null;
      try {
        final raw = jsonDecode(out.toDartString());
        if (raw is Map && raw["ok"] == true) {
          final pk = raw["public_key_hex"]?.toString().trim() ?? "";
          if (isValidPublicKeyHex(pk)) return pk;
        }
        return null;
      } catch (_) {
        return null;
      } finally {
        free(out);
      }
    } finally {
      calloc.free(p);
    }
  }

  /// Seal UTF-8 to a recipient secp256k1 public key (66 hex). Native symbol name is historical.
  static Map<String, dynamic> sealUtf8ToPublicKeyHex({
    required String recipientPublicKeyHex,
    required String plaintext,
  }) =>
      sealUtf8ToX25519Hex(
        recipientEncryptionPkHex: recipientPublicKeyHex,
        plaintext: plaintext,
      );

  static Map<String, dynamic> sealUtf8ToX25519Hex({
    required String recipientEncryptionPkHex,
    required String plaintext,
  }) {
    _ensure();
    final seal = _sealUtf8ToX25519Hex;
    final free = _stringFree;
    if (seal == null || free == null) {
      return {"ok": false, "error": "seal symbol missing"};
    }
    final a = recipientEncryptionPkHex.toNativeUtf8();
    final b = plaintext.toNativeUtf8();
    try {
      final out = seal(a, b);
      return _parseSmallJson(out, free);
    } finally {
      calloc.free(a);
      calloc.free(b);
    }
  }

  static Map<String, dynamic> openSealedCipherHex(String cipherHex) {
    _ensure();
    final open = _openSealedCipherHex;
    final free = _stringFree;
    if (open == null || free == null) {
      return {"ok": false, "error": "open_sealed symbol missing"};
    }
    final p = cipherHex.toNativeUtf8();
    try {
      final out = open(p);
      return _parseSmallJson(out, free);
    } finally {
      calloc.free(p);
    }
  }

  /// Returns the next event object, or `null` if none are queued.
  static Map<String, dynamic>? p2pPollEventMap() {
    final poll = _p2pPollEvent;
    final free = _stringFree;
    if (poll == null || free == null) return null;
    final out = poll();
    if (out == nullptr || out.address == 0) return null;
    try {
      final s = out.toDartString();
      final raw = jsonDecode(s);
      Map<String, dynamic>? map;
      if (raw is Map<String, dynamic>) {
        map = raw;
      } else if (raw is Map) {
        map = Map<String, dynamic>.from(raw);
      }
      if (map != null) {
        return map;
      }
      return null;
    } catch (_) {
      return null;
    } finally {
      free(out);
    }
  }

  /// Contacts list only — do not require invite/transcript symbols (P2P still works via daemon).
  static bool get isContactsStoreAvailable => _loaded && _contactsList != null;

  static bool get isNativeServiceAvailable =>
      isContactsStoreAvailable &&
      _transcriptLoadMerged != null &&
      _buildConnectInviteUri != null;

  static void _clearServiceSymbols() {
    _contactsList = null;
    _contactsUpsert = null;
    _contactsRemove = null;
    _contactsFind = null;
    _contactsMergeDiscovered = null;
    _contactsRecordPreview = null;
    _contactsClearUnread = null;
    _contactsSetTrust = null;
    _coordSettingsGet = null;
    _daemonSocketPath = null;
    _transcriptResolvePath = null;
    _transcriptLoadMerged = null;
    _transcriptSave = null;
    _transcriptAppend = null;
    _transcriptPatchDelivery = null;
    _transcriptPatchReadAck = null;
    _buildConnectInviteUri = null;
    _parseConnectInviteUri = null;
  }

  static void _loadServiceSymbols(DynamicLibrary lib) {
    _clearServiceSymbols();
    try {
      _contactsList = lib.lookupFunction<_NativePtrToPtr, _NativePtrToPtrDart>(
        "ghal_bol_ffi_contacts_list",
      );
    } catch (_) {}
    try {
      _contactsUpsert = lib.lookupFunction<_NativeTwoPtrToPtr, _NativeTwoPtrToPtrDart>(
        "ghal_bol_ffi_contacts_upsert",
      );
    } catch (_) {}
    try {
      _contactsRemove = lib.lookupFunction<_NativeTwoPtrToPtr, _NativeTwoPtrToPtrDart>(
        "ghal_bol_ffi_contacts_remove",
      );
    } catch (_) {}
    try {
      _contactsFind = lib.lookupFunction<_NativeTwoPtrToPtr, _NativeTwoPtrToPtrDart>(
        "ghal_bol_ffi_contacts_find",
      );
    } catch (_) {}
    try {
      _contactsMergeDiscovered =
          lib.lookupFunction<_NativeThreeStringsToPtr, _NativeThreeStringsToPtrDart>(
        "ghal_bol_ffi_contacts_merge_discovered_peer_id",
      );
    } catch (_) {}
    try {
      _contactsRecordPreview = lib.lookupFunction<_NativeTwoPtrToPtr, _NativeTwoPtrToPtrDart>(
        "ghal_bol_ffi_contacts_record_inbound_preview",
      );
    } catch (_) {}
    try {
      _contactsClearUnread = lib.lookupFunction<_NativeTwoStringsToPtr, _NativeTwoStringsToPtrDart>(
        "ghal_bol_ffi_contacts_clear_unread",
      );
    } catch (_) {}
    try {
      _coordSettingsGet = lib.lookupFunction<_NativePtrToPtr, _NativePtrToPtrDart>(
        "ghal_bol_ffi_coord_settings_get",
      );
    } catch (_) {}
    try {
      _daemonSocketPath = lib.lookupFunction<_NativePollPtr, _NativePollPtrDart>(
        "ghal_bol_ffi_daemon_socket_path",
      );
    } catch (_) {}
    try {
      _contactsSetTrust = lib.lookupFunction<_NativeTwoPtrToPtr, _NativeTwoPtrToPtrDart>(
        "ghal_bol_ffi_contacts_set_trust",
      );
    } catch (_) {}
    try {
      _transcriptResolvePath = lib.lookupFunction<_NativePtrToPtr, _NativePtrToPtrDart>(
        "ghal_bol_ffi_transcript_resolve_path",
      );
    } catch (_) {}
    try {
      _transcriptLoadMerged = lib.lookupFunction<_NativeTwoPtrToPtr, _NativeTwoPtrToPtrDart>(
        "ghal_bol_ffi_transcript_load_merged",
      );
    } catch (_) {}
    try {
      _transcriptSave = lib.lookupFunction<_NativeTwoPtrToPtr, _NativeTwoPtrToPtrDart>(
        "ghal_bol_ffi_transcript_save",
      );
    } catch (_) {}
    try {
      _transcriptAppend = lib.lookupFunction<_NativeTwoPtrToPtr, _NativeTwoPtrToPtrDart>(
        "ghal_bol_ffi_transcript_append_if_new",
      );
    } catch (_) {}
    try {
      _transcriptPatchDelivery = lib.lookupFunction<_NativeTwoPtrToPtr, _NativeTwoPtrToPtrDart>(
        "ghal_bol_ffi_transcript_patch_outgoing_delivery",
      );
    } catch (_) {}
    try {
      _transcriptPatchReadAck = lib.lookupFunction<_NativeTwoPtrToPtr, _NativeTwoPtrToPtrDart>(
        "ghal_bol_ffi_transcript_patch_inbound_read_ack_sent",
      );
    } catch (_) {}
    try {
      _buildConnectInviteUri = lib.lookupFunction<_NativePtrToPtr, _NativePtrToPtrDart>(
        "ghal_bol_ffi_build_connect_invite_uri",
      );
    } catch (_) {}
    try {
      _parseConnectInviteUri = lib.lookupFunction<_NativePtrToPtr, _NativePtrToPtrDart>(
        "ghal_bol_ffi_parse_connect_invite_uri",
      );
    } catch (_) {}
  }

  static Map<String, dynamic> _callJsonPtr(
    Pointer<Utf8> Function(Pointer<Utf8>)? fn,
    String arg,
  ) {
    final free = _stringFree;
    if (fn == null || free == null) {
      return {"ok": false, "error": "native service unavailable"};
    }
    final p = arg.toNativeUtf8();
    try {
      return _parseSmallJson(fn(p), free);
    } finally {
      calloc.free(p);
    }
  }

  static Map<String, dynamic> _callJson2Ptr(
    Pointer<Utf8> Function(Pointer<Utf8>, Pointer<Utf8>)? fn,
    String a,
    String b,
  ) {
    final free = _stringFree;
    if (fn == null || free == null) {
      return {"ok": false, "error": "native service unavailable"};
    }
    final pa = a.toNativeUtf8();
    final pb = b.toNativeUtf8();
    try {
      return _parseSmallJson(fn(pa, pb), free);
    } finally {
      calloc.free(pa);
      calloc.free(pb);
    }
  }

  static Map<String, dynamic> _callJson3Ptr(
    Pointer<Utf8> Function(Pointer<Utf8>, Pointer<Utf8>, Pointer<Utf8>)? fn,
    String a,
    String b,
    String c,
  ) {
    final free = _stringFree;
    if (fn == null || free == null) {
      return {"ok": false, "error": "native service unavailable"};
    }
    final pa = a.toNativeUtf8();
    final pb = b.toNativeUtf8();
    final pc = c.toNativeUtf8();
    try {
      return _parseSmallJson(fn(pa, pb, pc), free);
    } finally {
      calloc.free(pa);
      calloc.free(pb);
      calloc.free(pc);
    }
  }

  static List<Map<String, dynamic>> contactsList(String appNamespace) {
    final r = _callJsonPtr(_contactsList, appNamespace);
    if (r["ok"] != true) {
      AppLog.instance.w("Service", "contacts_list failed: ${r["error"]}");
      return [];
    }
    final raw = r["contacts"];
    if (raw is! List) return [];
    return raw
        .whereType<Map>()
        .map((e) => Map<String, dynamic>.from(e))
        .toList();
  }

  static Map<String, dynamic> contactsUpsert(
    String appNamespace,
    Map<String, dynamic> contact,
  ) {
    final r = _callJson2Ptr(_contactsUpsert, appNamespace, jsonEncode(contact));
    if (r["ok"] != true) {
      AppLog.instance.w("Service", "contacts_upsert failed: ${r["error"]}");
    }
    return r;
  }

  static bool contactsRemove(String appNamespace, Map<String, dynamic> contact) {
    return _callJson2Ptr(_contactsRemove, appNamespace, jsonEncode(contact))["ok"] == true;
  }

  static Map<String, dynamic>? contactsFind(String appNamespace, Map<String, dynamic> query) {
    final r = _callJson2Ptr(_contactsFind, appNamespace, jsonEncode(query));
    if (r["ok"] != true) return null;
    final c = r["contact"];
    if (c is Map) return Map<String, dynamic>.from(c);
    return null;
  }

  static bool contactsMergeDiscovered(
    String appNamespace,
    String publicKeyHex,
    String libp2pPeerId,
  ) =>
      _callJson3Ptr(_contactsMergeDiscovered, appNamespace, publicKeyHex, libp2pPeerId)["ok"] == true;

  static bool contactsRecordInboundPreview(String appNamespace, Map<String, dynamic> preview) =>
      _callJson2Ptr(_contactsRecordPreview, appNamespace, jsonEncode(preview))["ok"] == true;

  static bool contactsClearUnread(String appNamespace, String publicKeyHex) =>
      _callJson2Ptr(_contactsClearUnread, appNamespace, publicKeyHex)["ok"] == true;

  static Map<String, dynamic> contactsSetTrust(
    String appNamespace,
    Map<String, dynamic> trust,
  ) =>
      _callJson2Ptr(_contactsSetTrust, appNamespace, jsonEncode(trust));

  static Map<String, dynamic>? coordSettingsGet({required String appNamespace}) {
    final r = _callJsonPtr(_coordSettingsGet, appNamespace);
    if (r["ok"] != true) return null;
    return r;
  }

  static String? daemonSocketPath() {
    final free = _stringFree;
    final f = _daemonSocketPath;
    if (f == null || free == null) return null;
    final r = _parseSmallJson(f(), free);
    if (r["ok"] != true) return null;
    return r["path"]?.toString();
  }

  static String? transcriptResolvePath(String appNamespace) {
    final r = _callJsonPtr(_transcriptResolvePath, appNamespace);
    if (r["ok"] != true) return null;
    return r["path"]?.toString();
  }

  static List<Map<String, dynamic>> transcriptLoadMerged(
    String appNamespace,
    Map<String, dynamic> query,
  ) {
    final view = transcriptLoadThreadView(appNamespace, query);
    return view.lines;
  }

  static ({int revision, List<Map<String, dynamic>> lines}) transcriptLoadThreadView(
    String appNamespace,
    Map<String, dynamic> query,
  ) {
    final r = _callJson2Ptr(_transcriptLoadMerged, appNamespace, jsonEncode(query));
    if (r["ok"] != true) return (revision: 0, lines: <Map<String, dynamic>>[]);
    final revRaw = r["revision"];
    final revision = revRaw is num ? revRaw.toInt() : 0;
    final raw = r["lines"];
    if (raw is! List) return (revision: revision, lines: <Map<String, dynamic>>[]);
    final lines = raw
        .whereType<Map>()
        .map((e) => Map<String, dynamic>.from(e))
        .toList();
    return (revision: revision, lines: lines);
  }

  static bool transcriptSave(
    String appNamespace,
    String conversationKey,
    List<Map<String, dynamic>> lines,
  ) =>
      _callJson2Ptr(
        _transcriptSave,
        appNamespace,
        jsonEncode({"conversation_key": conversationKey, "lines": lines}),
      )["ok"] ==
      true;

  static bool transcriptAppendIfNew(
    String appNamespace,
    String conversationKey,
    Map<String, dynamic> line,
  ) =>
      _callJson2Ptr(
        _transcriptAppend,
        appNamespace,
        jsonEncode({"conversation_key": conversationKey, "line": line}),
      )["ok"] ==
      true;

  static bool transcriptPatchOutgoingDelivery(
    String appNamespace, {
    required String conversationKey,
    required String messageId,
    required String delivery,
  }) =>
      _callJson2Ptr(
        _transcriptPatchDelivery,
        appNamespace,
        jsonEncode({
          "conversation_key": conversationKey,
          "message_id": messageId,
          "delivery": delivery,
        }),
      )["ok"] ==
      true;

  static bool transcriptPatchInboundReadAckSent(
    String appNamespace, {
    required String conversationKey,
    required String messageId,
  }) =>
      _callJson2Ptr(
        _transcriptPatchReadAck,
        appNamespace,
        jsonEncode({"conversation_key": conversationKey, "message_id": messageId}),
      )["ok"] ==
      true;

  static String? buildConnectInviteUri(Map<String, dynamic> params) {
    if (_buildConnectInviteUri == null) return null;
    final r = _callJsonPtr(_buildConnectInviteUri, jsonEncode(params));
    if (r["ok"] != true) return null;
    return r["uri"]?.toString();
  }

  static Map<String, dynamic>? parseConnectInviteWire(String uri) {
    if (_parseConnectInviteUri == null) return null;
    final r = _callJsonPtr(_parseConnectInviteUri, uri);
    if (r["ok"] != true) return null;
    final wire = r["wire"];
    if (wire is Map) return Map<String, dynamic>.from(wire);
    return null;
  }

  static Map<String, dynamic> _parseSmallJson(
    Pointer<Utf8> outPtr,
    _NativeStringFreeDart free,
  ) {
    if (outPtr == nullptr || outPtr.address == 0) {
      return {"ok": false, "error": "null p2p response"};
    }
    try {
      final s = outPtr.toDartString();
      final raw = jsonDecode(s);
      if (raw is Map<String, dynamic>) return raw;
      if (raw is Map) return Map<String, dynamic>.from(raw);
      return {"ok": false, "error": "invalid p2p json"};
    } catch (e) {
      return {"ok": false, "error": "$e"};
    } finally {
      free(outPtr);
    }
  }
}
