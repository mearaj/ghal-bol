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

  static void _logCoordReachabilityHint(String err) {
    final e = err.toLowerCase();
    if (!e.contains("error sending request") &&
        !e.contains("connection refused") &&
        !e.contains("timed out") &&
        !e.contains("failed to connect")) {
      return;
    }
    P2pFlowLog.issue(
      "coord_unreachable",
      detail: err,
      check: "coord URL reachable from this device (HTTPS/ngrok); retry when network stable",
    );
  }

  static Future<List<String>> _lookupCoordBootstrap(List<SavedContact> contacts) async {
    if (!GhalBolCoord.isLookupEnabled) return [];
    final bootstrap = <String>{};
    for (final c in contacts) {
      final pk = _effectivePublicKeyHex(c);
      if (!isValidPublicKeyHex(pk)) continue;
      final lookup = await GhalBolCoord.lookupPeer(pk!);
      if (lookup["ok"] == true) {
        final addrs = lookup["bootstrap_peers"];
        if (addrs is List) {
          for (final a in addrs) {
            final s = a?.toString().trim() ?? "";
            if (s.isNotEmpty) bootstrap.add(s);
          }
        }
      } else {
        final err = lookup["error"]?.toString() ?? "lookup failed";
        final e = err.toLowerCase();
        if (e.contains("coord base url not set")) {
          P2pFlowLog.coord("lookup_skip", {
            "peer": P2pFlowLog.shortPk(pk),
            "reason": "coord_url_not_set",
          });
        } else if (e.contains("404") || e.contains("not found")) {
          P2pFlowLog.coord("lookup_miss", {
            "peer": P2pFlowLog.shortPk(pk),
            "reason": "peer_not_on_server",
          });
        } else {
          P2pFlowLog.coord("lookup_fail", {
            "peer": P2pFlowLog.shortPk(pk),
            "error": err,
          });
          _logCoordReachabilityHint(err);
        }
      }
    }
    return bootstrap.toList();
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

  /// Coord lookup + hot dial only (no dm_peers fingerprint churn). Call after peer may have registered.
  static Future<void> refreshCoordDial(
    List<SavedContact> contacts, {
    required String appNamespace,
  }) async {
    if (!GhalBolP2p.isAvailable || !await GhalBolP2p.isRunning()) return;
    if (_dmPeersFingerprint(contacts) == "[]") return;
    final bootstrap = await _lookupCoordBootstrap(contacts);
    if (bootstrap.isEmpty) {
      P2pFlowLog.coord("dial_skip", {"reason": "no_coord_addrs"});
      return;
    }
    P2pFlowLog.coord("dial_start", {"addrs": bootstrap.length.toString()});
    final r = await GhalBolP2p.dialBootstrapPeers(bootstrap);
    if (r["ok"] != true) {
      P2pFlowLog.issue("coord_dial_failed", detail: r["error"]?.toString());
    } else {
      P2pFlowLog.coord("dial_ok");
    }
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
    bool lookupBootstrap = false,
  }) async {
    final inFlight = _syncInFlight;
    if (inFlight != null) {
      AppLog.instance.d("P2P", "sync_contacts: coalesced (start in flight)");
      return inFlight;
    }
    final run = _syncContactsImpl(
      contacts,
      appNamespace: appNamespace,
      lookupBootstrap: lookupBootstrap,
    );
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
    bool lookupBootstrap = false,
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
    // Start the native node first (mDNS browse + listen). Coord lookup is fallback only —
    // do not block p2p_start on HTTP (often fails on phone before the peer is registered).
    var bootstrap = <String>[];
    if (dmFp != "[]" && lookupBootstrap) {
      bootstrap = await _lookupCoordBootstrap(contacts);
      P2pFlowLog.coord("lookup_done", {
        "peers": contacts.length.toString(),
        "bootstrap": bootstrap.length.toString(),
      });
    }

    var cfg = await _buildConfig(contacts, bootstrapPeers: bootstrap);
    cfg = await _configWithNamespace(cfg, appNamespace);

    final ns = appNamespace.trim();
    final withPk = contacts.where((c) => isValidPublicKeyHex(_effectivePublicKeyHex(c))).length;
    P2pFlowLog.step("sync_contacts", {
      "total": contacts.length.toString(),
      "with_pk": withPk.toString(),
      "ns": ns,
      "lookup_bootstrap": lookupBootstrap.toString(),
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
