import "dart:async";
import "dart:convert";
import "dart:io" show Platform;

import "app_log.dart";
import "call/call_controller.dart";
import "user_flow_log.dart";
import "chat_transcript_store.dart";
import "ghal_bol_coord.dart";
import "ghal_bol_p2p.dart";
import "ghal_bol_listener_foreground.dart";
import "public_key_hex.dart";
import "saved_contact.dart";
import "session_credentials.dart";

/// Starts or reconfigures the native P2P node for all saved contacts.
class P2pNetworkCoordinator {
  P2pNetworkCoordinator._();

  static final Map<String, String> _hotRegisteredFingerprints = {};
  static Future<Map<String, dynamic>>? _syncInFlight;
  static String? _lastSyncFingerprint;
  static String? _lastSyncNamespace;
  static bool _needsFullP2pStart = true;
  static String? _effectivePublicKeyHex(SavedContact c) {
    final pk = c.publicKeyHex.trim().toLowerCase();
    return isValidPublicKeyHex(pk) ? pk : null;
  }

  /// DM identity fingerprint — keyed by public key when available (authoritative).
  static String dmPeersFingerprint(List<SavedContact> contacts) =>
      _dmPeersFingerprint(contacts);

  static String _dmPeersFingerprint(List<SavedContact> contacts) {
    final parts = <String>[];
    for (final c in contacts) {
      final pk = _effectivePublicKeyHex(c);
      if (isValidPublicKeyHex(pk)) {
        parts.add("k:$pk");
      }
    }
    parts.sort();
    return jsonEncode(parts);
  }

  /// Fast path — no HTTP (matches [ghal_bol_workspace-one]).
  static Future<Map<String, dynamic>> _buildConfig(
    List<SavedContact> contacts, {
    List<String> bootstrapPeers = const [],
  }) async {
    final dmPeers = <Map<String, dynamic>>[];
    for (final c in contacts) {
      final pk = _effectivePublicKeyHex(c);
      if (isValidPublicKeyHex(pk)) {
        dmPeers.add(<String, dynamic>{"public_key_hex": pk!.toLowerCase()});
      }
    }
    return {
      "bootstrap_peers": bootstrapPeers,
      "dm_peers": dmPeers,
      ...await GhalBolCoord.p2pConfigFields(),
    };
  }

  static Future<Map<String, dynamic>> _configWithNamespace(
    Map<String, dynamic> cfg,
    String appNamespace,
  ) async {
    final ns = appNamespace.trim();
    if (ns.isNotEmpty) {
      cfg["app_namespace"] = ns;
      try {
        cfg["transcript_path"] = await ChatTranscriptStore.resolvePathForNamespace(ns);
      } catch (_) {}
    }
    return cfg;
  }

  /// Nudge native DM discovery after a peer may have registered on coord.
  /// HTTP coord lookup + dial live in Rust (`chat_server.rs` / `coord_runtime.rs`) only.
  static Future<void> refreshCoordDial(
    List<SavedContact> contacts, {
    required String appNamespace,
  }) async {
    if (!GhalBolP2p.isAvailable || !await GhalBolP2p.isRunning()) return;
    if (_dmPeersFingerprint(contacts) == "[]") return;
    await registerContacts(contacts);
    P2pFlowLog.coord("discovery_kick", {"reason": "native_register_dm_peer"});
  }

  static Future<void> registerContacts(Iterable<SavedContact> contacts) async {
    if (!GhalBolP2p.isAvailable || !await GhalBolP2p.isRunning()) return;
    var registered = 0;
    for (final c in contacts) {
      final pk = _effectivePublicKeyHex(c);
      if (!isValidPublicKeyHex(pk)) continue;
      final pkNorm = pk!.toLowerCase();
      final fp = "k:$pkNorm";
      if (_hotRegisteredFingerprints[pkNorm] == fp) continue;
      _hotRegisteredFingerprints[pkNorm] = fp;
      await GhalBolP2p.registerDmPeer(pkNorm);
      registered++;
    }
    if (registered > 0) {
      AppLog.instance.trace(
        "register_dm_peer",
        "hot-registered $registered peer(s) on running node",
      );
    }
  }

  static Future<Map<String, dynamic>> syncContacts(
    List<SavedContact> contacts, {
    required String appNamespace,
  }) async {
    final inFlight = _syncInFlight;
    if (inFlight != null) {
      AppLog.instance.d("P2P", "sync_contacts: coalesced (start in flight)");
      return inFlight;
    }
    final run = _syncContactsImpl(contacts, appNamespace: appNamespace);
    _syncInFlight = run;
    try {
      return await run;
    } finally {
      if (identical(_syncInFlight, run)) {
        _syncInFlight = null;
      }
    }
  }

  static Future<Map<String, dynamic>> _syncContactsImpl(
    List<SavedContact> contacts, {
    required String appNamespace,
  }) async {
    if (!GhalBolP2p.isAvailable) {
      P2pFlowLog.issue("p2p_unavailable");
      return {"ok": false, "error": "native p2p unavailable"};
    }
    if (GhalBolP2p.usesDaemon && !await SessionCredentials.ensureDaemonUnlocked()) {
      P2pFlowLog.issue(
        "daemon_not_unlocked",
        check: "Session step=login_submit then Daemon re_unlock",
      );
      return {"ok": false, "error": "identity not unlocked"};
    }

    final dmFp = _dmPeersFingerprint(contacts);
    // Peer coord lookup + dial live only in ghal_bol (`chat_server.rs` / `coord_runtime.rs`).
    var cfg = await _buildConfig(contacts);
    cfg = await _configWithNamespace(cfg, appNamespace);

    final ns = appNamespace.trim();
    final withPk = contacts.where((c) => isValidPublicKeyHex(_effectivePublicKeyHex(c))).length;
    P2pFlowLog.step("sync_contacts", {
      "total": contacts.length.toString(),
      "with_pk": withPk.toString(),
      "ns": ns,
    });

    await ghalBolListenerForegroundEnsureStarted();

    final running = await GhalBolP2p.isRunning();
    final fpUnchanged = running &&
        !_needsFullP2pStart &&
        _lastSyncNamespace == ns &&
        _lastSyncFingerprint == dmFp;
    if (fpUnchanged) {
      await registerContacts(contacts);
      if (contacts.any((c) => isValidPublicKeyHex(_effectivePublicKeyHex(c)))) {
        unawaited(refreshCoordDial(contacts, appNamespace: ns));
      }
      unawaited(CallController.instance.syncActiveCallFromNative());
      P2pFlowLog.step("sync_contacts_skip", {"reason": "hot_register_only"});
      return {"ok": true, "already_running": true};
    }

    P2pFlowLog.step("p2p_start");
    final r = await GhalBolP2p.startJson(cfg);
    if (r["already_running"] == true) {
      P2pFlowLog.step("p2p_start", {"already_running": "true"});
    }
    if (r["ok"] == true) {
      _needsFullP2pStart = false;
      _lastSyncNamespace = ns;
      _lastSyncFingerprint = dmFp;
      _hotRegisteredFingerprints.clear();
      if (r["already_running"] != true) {
        final ready = await GhalBolP2p.waitNodeReady(
          timeout: Platform.isAndroid
              ? const Duration(seconds: 30)
              : const Duration(seconds: 8),
        );
        if (!ready) {
          P2pFlowLog.issue(
            "node_ready_timeout",
            check: "grep P2P step=node_ready; mDNS vs coord",
          );
          return {"ok": false, "error": "p2p node failed to start"};
        }
      }
      await registerContacts(contacts);
      unawaited(GhalBolCoord.registerAndVerifyAfterP2pUp());
      P2pFlowLog.step("node_running");
      if (contacts.any((c) => isValidPublicKeyHex(_effectivePublicKeyHex(c)))) {
        unawaited(refreshCoordDial(contacts, appNamespace: ns));
      }
      unawaited(CallController.instance.syncActiveCallFromNative());
    } else {
      P2pFlowLog.issue("p2p_start_failed", detail: r["error"]?.toString());
    }
    return r;
  }

  /// After unlock — next [syncContacts] must run full [p2p_start].
  static void markSessionRefresh() {
    P2pFlowLog.step("session_refresh");
    _needsFullP2pStart = true;
    _lastSyncFingerprint = null;
    _lastSyncNamespace = null;
    _hotRegisteredFingerprints.clear();
  }

  static void invalidate() {
    P2pFlowLog.step("coordinator_invalidated");
    _hotRegisteredFingerprints.clear();
    _syncInFlight = null;
    markSessionRefresh();
  }
}
