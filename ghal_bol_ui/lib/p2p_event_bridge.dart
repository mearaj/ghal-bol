import "dart:async";

import "package:ghal_bol_ui/app_log.dart";
import "package:ghal_bol_ui/user_flow_log.dart";
import "package:ghal_bol_ui/call/call_controller.dart";
import "package:ghal_bol_ui/contact_store.dart";
import "package:ghal_bol_ui/ghal_bol_constants.dart";
import "package:ghal_bol_ui/ghal_bol_p2p.dart";
import "package:ghal_bol_ui/session_credentials.dart";
import "package:ghal_bol_ui/src/ghal_bol_daemon_client_io.dart"
    if (dart.library.html) "package:ghal_bol_ui/src/ghal_bol_daemon_client_stub.dart";
import "package:ghal_bol_ui/src/ghal_bol_ffi_result.dart";
import "package:ghal_bol_ui/p2p_network_coordinator.dart";
import "package:ghal_bol_ui/public_key_hex.dart";

/// App-wide P2P poll loop. DM → contacts/transcript side effects run in **`ghal_bol`**.
class P2pEventBridge {
  P2pEventBridge._();

  static final P2pEventBridge instance = P2pEventBridge._();

  Timer? _poll;
  Timer? _heartbeat;
  Timer? _coordDialDebounce;
  String _appNs = kGhalBolAndroidLibraryNamespace;
  bool _networkBootstrapOk = false;
  Future<void>? _bootstrapFuture;
  final Set<String> _streamReadyPeers = {};
  final List<void Function(Map<String, dynamic> ev)> _listeners = [];
  final Set<String> _identifiedPeersHotRegistered = {};
  bool _nodeReady = false;
  final Set<String> _lanIpv4Candidates = {};
  int _drainEvents = 0;
  int _drainEmptyPolls = 0;
  DateTime? _lastHeartbeatAt;
  DateTime? _lastEmptyPollLogAt;
  Future<void>? _p2pRecoverInFlight;
  DateTime? _lastRecoverAttemptAt;

  /// Latest hub room target; pump applies until it matches native (coalesces rapid enter/leave).
  String? _foregroundDesired;
  Future<void>? _foregroundPumpFuture;
  bool? _appAckReadDesired;

  bool get isRunning => _poll != null;
  /// True after native `node_ready` (DM transport up). Used for share QR — not listen addrs.
  bool get isNodeReady => _nodeReady;
  /// Best LAN IPv4 from `listening` events (diagnostics / future use).
  String? get primaryLanIpv4 => _pickBestLanIpv4(_lanIpv4Candidates);
  String get appNamespace => _appNs;

  bool isStreamReady(String peerOrPublicKeyHex) {
    final trimmed = peerOrPublicKeyHex.trim();
    if (trimmed.isEmpty) return false;
    final lower = trimmed.toLowerCase();
    if (lower.length == kSecp256k1PublicKeyHexLen &&
        _streamReadyPeers.contains(lower)) {
      return true;
    }
    if (isValidPublicKeyHex(trimmed) && _streamReadyPeers.contains(trimmed.toLowerCase())) {
      return true;
    }
    return false;
  }

  void addListener(void Function(Map<String, dynamic> ev) listener) {
    if (!_listeners.contains(listener)) {
      _listeners.add(listener);
    }
  }

  void removeListener(void Function(Map<String, dynamic> ev) listener) {
    _listeners.remove(listener);
  }

  /// Hub sets which contact is the open chat room (`public_key_hex` → native foreground).
  void setForegroundConversation(String? publicKeyHex) {
    final p = publicKeyHex?.trim().toLowerCase() ?? "";
    final wantPeer = isValidPublicKeyHex(p) ? p : null;
    if (_foregroundDesired != wantPeer) {
      SessionFlowLog.step("foreground_desired", {
        "peer": wantPeer ?? "(none)",
        "was": _foregroundDesired ?? "(none)",
      });
    }
    _foregroundDesired = wantPeer;
    final pump = (_foregroundPumpFuture ?? Future<void>.value()).then((_) async {
      while (true) {
        final want = _foregroundDesired;
        await _applyForegroundPeer(want);
        if (_foregroundDesired == want) break;
      }
    });
    _foregroundPumpFuture = pump.catchError((_) {});
  }

  /// Wait until [setForegroundConversation] has applied in native (ordering for ack_read gate).
  Future<void> awaitForegroundApplied() async {
    final f = _foregroundPumpFuture;
    if (f != null) await f.catchError((_) {});
  }

  Future<void> _applyForegroundPeer(String? publicKeyHex) async {
    if (!GhalBolP2p.isAvailable) return;
    if (GhalBolP2p.usesDaemon && !await GhalBolDaemonClient.probeDaemon()) {
      await GhalBolDaemonClient.ensureDaemonRunning();
    }
    if (!await GhalBolP2p.isRunning()) return;
    final r = await GhalBolP2p.setForegroundPeer(publicKeyHex);
    if (r["ok"] != true) {
      P2pFlowLog.issue("set_foreground_failed", detail: r["error"]?.toString());
    } else {
      SessionFlowLog.step("foreground_applied", {
        "pk": publicKeyHex != null && isValidPublicKeyHex(publicKeyHex)
            ? P2pFlowLog.shortPk(publicKeyHex)
            : "(none)",
      });
    }
  }

  /// Starts poll loop immediately; P2P bootstrap runs in the background.
  Future<void> ensureStarted(GhalBolIdentityResult session) async {
    GhalBolP2p.pollEventDispatcher = dispatchPolledEvent;
    _appNs = session.appNamespace?.trim().isNotEmpty == true
        ? session.appNamespace!.trim()
        : kGhalBolAndroidLibraryNamespace;
    SessionFlowLog.step("bridge_start", {
      "ns": _appNs,
      "daemon": GhalBolP2p.usesDaemon.toString(),
    });
    AppLog.instance.trace("session_start", "poll bridge + network bootstrap");
    _poll ??= Timer.periodic(const Duration(milliseconds: 200), (_) {
      unawaited(_drain());
    });
    _heartbeat ??= Timer.periodic(const Duration(minutes: 2), (_) {
      unawaited(_logSessionHeartbeat());
    });
    if (_networkBootstrapOk) return;
    if (_bootstrapFuture != null) return;
    _bootstrapFuture = _bootstrapNetworkOnce().whenComplete(() {
      _bootstrapFuture = null;
    });
    unawaited(_bootstrapFuture);
  }

  Future<void> _bootstrapNetworkOnce() async {
    try {
      if (!GhalBolP2p.isAvailable) {
        P2pFlowLog.issue("bootstrap_unavailable");
        return;
      }
      if (GhalBolP2p.usesDaemon && !await SessionCredentials.ensureDaemonUnlocked()) {
        return;
      }
      P2pFlowLog.step("bootstrap_start");
      final contacts = await ContactStore.listContacts(_appNs);
      final r = await P2pNetworkCoordinator.syncContacts(
        contacts,
        appNamespace: _appNs,
      );
      if (r["ok"] != true) {
        P2pFlowLog.issue("bootstrap_sync_failed", detail: r["error"]?.toString());
        return;
      }
      _networkBootstrapOk = await GhalBolP2p.isRunning();
      P2pFlowLog.step("bootstrap_ok", {
        "running": _networkBootstrapOk.toString(),
      });
      if (_networkBootstrapOk) {
        _reapplyDeferredSessionRpc();
      }
    } catch (e, st) {
      AppLog.instance.e("P2P", "bootstrap exception", e, st);
    }
  }

  /// Re-run hub foreground / ack_read after P2P comes up (unlock is not blocked on this).
  void _reapplyDeferredSessionRpc() {
    unawaited(() async {
      if (_appAckReadDesired != null) {
        await _applyAppAckReadEnabled();
      }
      final fg = _foregroundDesired;
      if (fg != null) {
        await _applyForegroundPeer(fg);
      }
    }());
  }

  /// Protonet-style gate: no read receipts while backgrounded or hub UI torn down.
  Future<void> setAppAckReadEnabled(bool enabled) async {
    SessionFlowLog.step("ack_read_desired", {"enabled": enabled.toString()});
    _appAckReadDesired = enabled;
    await _applyAppAckReadEnabled();
  }

  Future<void> _applyAppAckReadEnabled() async {
    final enabled = _appAckReadDesired;
    if (enabled == null) return;
    if (!GhalBolP2p.isAvailable) return;
    if (GhalBolP2p.usesDaemon && !await GhalBolDaemonClient.probeDaemon()) {
      if (!enabled) return;
      await GhalBolDaemonClient.ensureDaemonRunning();
    }
    if (!await GhalBolP2p.isRunning()) return;
    final r = await GhalBolP2p.setAppAckReadEnabled(enabled);
    if (r["ok"] != true) {
      P2pFlowLog.issue("set_ack_read_failed", detail: r["error"]?.toString());
    } else {
      SessionFlowLog.step("ack_read_applied", {"enabled": enabled.toString()});
    }
  }

  Future<void> _logSessionHeartbeat() async {
    final now = DateTime.now();
    if (_lastHeartbeatAt != null &&
        now.difference(_lastHeartbeatAt!).inMinutes < 2) {
      return;
    }
    _lastHeartbeatAt = now;
    final running = await GhalBolP2p.isRunning();
    SessionFlowLog.step("heartbeat", {
      "running": running.toString(),
      "node_ready": _nodeReady.toString(),
      "bootstrap_ok": _networkBootstrapOk.toString(),
      "stream_ready_count": _streamReadyPeers.length.toString(),
      "poll_events": _drainEvents.toString(),
    });
    if (!running && _networkBootstrapOk) {
      unawaited(recoverP2pIfNeeded());
    }
  }

  /// After daemon restart or broken socket, re-run unlock-era `p2p_start` (same data dir).
  Future<void> recoverP2pIfNeeded() async {
    if (!_networkBootstrapOk || !GhalBolP2p.isAvailable) return;
    if (_p2pRecoverInFlight != null) return _p2pRecoverInFlight;
    final now = DateTime.now();
    final last = _lastRecoverAttemptAt;
    final backoffSec = SessionCredentials.hasPassword ? 8 : 60;
    if (last != null && now.difference(last).inSeconds < backoffSec) return;
    _lastRecoverAttemptAt = now;

    Future<void> run() async {
      if (await GhalBolP2p.isRunning()) return;
      if (GhalBolP2p.usesDaemon) {
        final up = await GhalBolDaemonClient.probeDaemon(force: true);
        if (!up) {
          await GhalBolDaemonClient.ensureDaemonRunning();
        }
        if (!await GhalBolDaemonClient.probeDaemon(force: true)) return;
        if (!await SessionCredentials.ensureDaemonUnlocked()) return;
      }
      P2pFlowLog.step("recover_start");
      _nodeReady = false;
      _streamReadyPeers.clear();
      try {
        final contacts = await ContactStore.listContacts(_appNs);
        final r = await P2pNetworkCoordinator.syncContacts(
          contacts,
          appNamespace: _appNs,
        );
        if (r["ok"] != true) {
          P2pFlowLog.issue("recover_sync_failed", detail: r["error"]?.toString());
          return;
        }
        final ready = await GhalBolP2p.waitNodeReady(
          timeout: const Duration(seconds: 12),
        );
        _nodeReady = ready;
        _networkBootstrapOk = ready;
        if (ready) {
          P2pFlowLog.step("recover_ok");
          final fg = _foregroundDesired;
          if (fg != null && fg.isNotEmpty) {
            await setAppAckReadEnabled(true);
            setForegroundConversation(fg);
          }
        } else {
          P2pFlowLog.issue("recover_node_not_ready");
        }
      } catch (e, st) {
        AppLog.instance.e("P2P", "recover failed", e, st);
      }
    }
    _p2pRecoverInFlight = run();
    try {
      await _p2pRecoverInFlight;
    } finally {
      _p2pRecoverInFlight = null;
    }
  }

  void stop() {
    AppLog.instance.i("Session", "bridge stop");
    if (identical(GhalBolP2p.pollEventDispatcher, dispatchPolledEvent)) {
      GhalBolP2p.pollEventDispatcher = null;
    }
    _poll?.cancel();
    _poll = null;
    _heartbeat?.cancel();
    _heartbeat = null;
    _coordDialDebounce?.cancel();
    _coordDialDebounce = null;
    _networkBootstrapOk = false;
    _bootstrapFuture = null;
    _foregroundDesired = null;
    _foregroundPumpFuture = null;
    _appAckReadDesired = null;
    _streamReadyPeers.clear();
    _identifiedPeersHotRegistered.clear();
    _listeners.clear();
    _nodeReady = false;
    unawaited(setAppAckReadEnabled(false));
  }

  static const int _maxEventsPerDrain = 32;
  Future<void>? _drainChain;

  void drainNow() {
    unawaited(_drain(maxEvents: 48));
  }

  Future<void> _drain({int maxEvents = _maxEventsPerDrain}) {
    Future<void> run() async {
      if (!GhalBolP2p.isAvailable) return;
      final savedDispatcher = GhalBolP2p.pollEventDispatcher;
      GhalBolP2p.pollEventDispatcher = null;
      final pending = <Map<String, dynamic>>[];
      try {
        for (var n = 0; n < maxEvents; n++) {
          final ev = await GhalBolP2p.pollEventMap();
          if (ev == null) {
            _drainEmptyPolls++;
            _maybeLogEmptyPollStreak();
            if (_drainEmptyPolls >= 3 && _networkBootstrapOk) {
              unawaited(recoverP2pIfNeeded());
            }
            break;
          }
          _drainEmptyPolls = 0;
          pending.add(ev);
        }
      } finally {
        GhalBolP2p.pollEventDispatcher = savedDispatcher;
      }
      if (pending.isEmpty) return;
      pending.sort((a, b) {
        final ac = a["kind"]?.toString() == "call_signal" ? 0 : 1;
        final bc = b["kind"]?.toString() == "call_signal" ? 0 : 1;
        return ac.compareTo(bc);
      });
      final batch = pending.length;
      _drainEvents += batch;
      for (final ev in pending) {
        dispatchPolledEvent(ev);
      }
      if (batch > 0) {
        if (batch >= maxEvents) {
          AppLog.instance.flow(
            "Session",
            "poll drain saturated batch=$batch totalEvents=$_drainEvents",
          );
        }
      }
    }
    final next = (_drainChain ?? Future<void>.value()).then((_) => run());
    _drainChain = next.catchError((_) {});
    return next;
  }

  /// Called from [GhalBolP2p.pollEventMap] so [waitNodeReady] cannot steal `node_ready`.
  void dispatchPolledEvent(Map<String, dynamic> ev) {
    _dispatch(ev);
  }

  void _dispatch(Map<String, dynamic> ev) {
    _handleCore(ev);
    CallController.instance.handlePollEvent(ev);
    for (final l in List<void Function(Map<String, dynamic>)>.from(_listeners)) {
      l(ev);
    }
  }

  void _handleCore(Map<String, dynamic> ev) {
    final kind = ev["kind"]?.toString();
    if (ev["stores_updated"] == true) {
      AppLog.instance.flow("DM/store", "UI refresh: stores_updated kind=$kind");
      final msgKind = ev["msg_kind"]?.toString() ?? "";
      // DESIGN.md / AGENTS.md: roster bump on peer_identified only — not every inbound text
      // (each text was forcing sync_contacts → p2p_register_dm_peer storm on the main RPC socket).
      if (kind == "peer_identified") {
        ContactStore.rosterChangeCount.value++;
        _scheduleCoordDialIfNeeded();
      } else if (kind == "dm_message" && msgKind == "text") {
        ContactStore.previewChangeCount.value++;
      } else {
        ContactStore.bumpListFromPoll();
      }
    }
    if (kind == "listening") {
      final ip = _ipv4FromListeningMultiaddr(ev["multiaddr"]?.toString() ?? "");
      if (ip != null) _lanIpv4Candidates.add(ip);
    }
    if (kind == "node_ready") {
      _nodeReady = true;
      _networkBootstrapOk = true;
      P2pFlowLog.step("node_ready");
      _reapplyDeferredSessionRpc();
      unawaited(_coordDialIfNeeded());
    } else if (kind == "node_stopped") {
      _nodeReady = false;
      _lanIpv4Candidates.clear();
      P2pFlowLog.issue("node_stopped", detail: ev["error"]?.toString());
    }
    if (kind == "peer_connected" || kind == "chat_ready") {
      final pk = publicKeyHexFromEvent(ev);
      if (pk.isNotEmpty && _streamReadyPeers.add(pk.toLowerCase())) {
        P2pFlowLog.step("stream_ready", {
          "peer": P2pFlowLog.shortPk(pk),
          "kind": kind ?? "?",
          "count": _streamReadyPeers.length.toString(),
        });
      }
    }
    if (kind == "peer_disconnected") {
      final pk = publicKeyHexFromEvent(ev);
      if (pk.isNotEmpty && _streamReadyPeers.remove(pk.toLowerCase())) {
        P2pFlowLog.step("stream_down", {
          "peer": P2pFlowLog.shortPk(pk),
          "count": _streamReadyPeers.length.toString(),
        });
      }
    }
    if (kind == "peer_identified") {
      final pk = publicKeyHexFromEvent(ev);
      if (pk.isNotEmpty && _identifiedPeersHotRegistered.add(pk)) {
        unawaited(_registerLearnedContact(pk));
      }
    }
  }

  /// Peer may have registered on the coord server — debounced lookup+dial (not full sync).
  void _scheduleCoordDialIfNeeded() {
    _coordDialDebounce?.cancel();
    _coordDialDebounce = Timer(const Duration(seconds: 3), () {
      unawaited(_coordDialIfNeeded());
    });
  }

  Future<void> _coordDialIfNeeded() async {
    try {
      final contacts = await ContactStore.listContacts(_appNs);
      await P2pNetworkCoordinator.refreshCoordDial(
        contacts,
        appNamespace: _appNs,
      );
    } catch (e, st) {
      AppLog.instance.d("P2P", "coord dial skipped: $e $st");
    }
  }

  Future<void> _registerLearnedContact(String publicKeyHex) async {
    final c = await ContactStore.findByPublicKey(
      appNamespace: _appNs,
      publicKeyHex: publicKeyHex,
    );
    if (c != null && c.hasFullKeys) {
      AppLog.instance.flow(
        "Session",
        "peer_identified → hot-register dm_peer pk=${publicKeyHex.substring(0, 8)}…",
      );
      await P2pNetworkCoordinator.registerContacts([c]);
    } else {
      AppLog.instance.w(
        "Session",
        "peer_identified but no contact row with keys pk=${publicKeyHex.substring(0, 8)}…",
      );
    }
  }

  void _maybeLogEmptyPollStreak() {
    if (_drainEmptyPolls < 25 || !_nodeReady) return;
    final now = DateTime.now();
    final last = _lastEmptyPollLogAt;
    if (last != null && now.difference(last).inMinutes < 5) return;
    _lastEmptyPollLogAt = now;
    AppLog.instance.d(
      "Session",
      "poll idle ×$_drainEmptyPolls (no queued events)",
    );
  }

  static String? _ipv4FromListeningMultiaddr(String multiaddr) {
    final m = RegExp(r"/ip4/(\d+\.\d+\.\d+\.\d+)/").firstMatch(multiaddr);
    if (m == null) return null;
    final ip = m.group(1)!;
    if (ip.startsWith("127.")) return null;
    if (ip.startsWith("169.254.")) return null;
    final parts = ip.split(".");
    if (parts.length == 4) {
      final second = int.tryParse(parts[1]) ?? -1;
      if (parts[0] == "172" && second >= 16 && second <= 31) return null;
    }
    return ip;
  }

  static String? _pickBestLanIpv4(Set<String> candidates) {
    if (candidates.isEmpty) return null;
    int score(String ip) {
      if (ip.startsWith("192.168.")) return 3;
      if (ip.startsWith("10.")) return 2;
      return 1;
    }
    String? best;
    var bestScore = -1;
    for (final ip in candidates) {
      final s = score(ip);
      if (s > bestScore) {
        bestScore = s;
        best = ip;
      }
    }
    return best;
  }
}
