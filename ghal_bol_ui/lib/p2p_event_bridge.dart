import "dart:async";

import "package:flutter/foundation.dart" show defaultTargetPlatform, kIsWeb, TargetPlatform;
import "package:ghal_bol_ui/app_log.dart";
import "package:ghal_bol_ui/user_flow_log.dart";
import "package:ghal_bol_ui/call/call_incoming_alert.dart";
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
  Timer? _linuxWakePoll;
  Timer? _coordDialDebounce;
  Future<void>? _coordDialInFlight;
  String _appNs = kGhalBolAppNamespace;
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
  Future<void>? _uiSessionPumpFuture;
  /// App interactive + visible (resumed, not inactive/paused). Gates read receipts with room.
  bool _uiVisibleDesired = false;

  /// Poll-derived display hints only — never use for ack/dial/send policy (native owns that).
  bool get isNodeReady => _nodeReady;

  /// Linux only: user pressed window **close (X)** — not minimize, not alt-tab unfocus.
  bool _linuxWindowClosedByUser = false;
  final List<void Function(bool closedByUser)> _linuxWindowCloseListeners = [];

  bool get linuxWindowClosedByUser => _linuxWindowClosedByUser;

  void addLinuxWindowCloseListener(void Function(bool closedByUser) listener) {
    if (!_linuxWindowCloseListeners.contains(listener)) {
      _linuxWindowCloseListeners.add(listener);
    }
  }

  void removeLinuxWindowCloseListener(void Function(bool closedByUser) listener) {
    _linuxWindowCloseListeners.remove(listener);
  }

  void _notifyLinuxWindowCloseListeners(bool closedByUser) {
    for (final l
        in List<void Function(bool closedByUser)>.from(_linuxWindowCloseListeners)) {
      l(closedByUser);
    }
  }

  bool get isRunning => _poll != null;
  /// Best LAN IPv4 from `listening` events (diagnostics / future use).
  String? get primaryLanIpv4 => _pickBestLanIpv4(_lanIpv4Candidates);
  String get appNamespace => _appNs;

  bool isStreamReady(String peerOrPublicKeyHex) {
    final trimmed = peerOrPublicKeyHex.trim();
    if (trimmed.isEmpty) return false;
    if (isValidPublicKeyHex(trimmed) &&
        _streamReadyPeers.contains(trimmed.toLowerCase())) {
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
    _scheduleUiSessionApply();
  }

  /// App resumed / paused / inactive — gates **new** read receipts without clearing room on brief inactive.
  void setUiVisible(bool visible) {
    if (_uiVisibleDesired == visible) return;
    SessionFlowLog.step("ui_visible_desired", {"visible": visible.toString()});
    _uiVisibleDesired = visible;
    _scheduleUiSessionApply();
  }

  /// Re-send the current desired UI session to native (`p2p_sync_ui_session`).
  ///
  /// Use when the hub still shows the room open but `:p2p` read gate may be stale
  /// (Linux desktop) — not a per-frame retry; call from debounced poll hooks only.
  void nudgeUiSessionSnapshot() {
    if (!GhalBolP2p.isAvailable) return;
    unawaited(GhalBolP2p.nudgeReadCatchup());
  }

  /// Wait until [setForegroundConversation] / [setUiVisible] has applied in native.
  Future<void> awaitForegroundApplied() => awaitUiSessionApplied();

  Future<void> awaitUiSessionApplied() async {
    final f = _uiSessionPumpFuture;
    if (f != null) await f.catchError((_) {});
  }

  void _scheduleUiSessionApply() {
    final pump = (_uiSessionPumpFuture ?? Future<void>.value()).then((_) async {
      while (true) {
        // Coalesce hub startup close→open (and layout flicker) into one native snapshot.
        await Future<void>.delayed(Duration.zero);
        var wantRoom = _foregroundDesired;
        var wantVisible = _uiVisibleDesired;
        if (wantRoom == null) {
          await Future<void>.delayed(const Duration(milliseconds: 32));
          if (_foregroundDesired != null) {
            wantRoom = _foregroundDesired;
            wantVisible = _uiVisibleDesired;
          }
        }
        await _applyUiSession(wantVisible, wantRoom);
        if (_foregroundDesired == wantRoom && _uiVisibleDesired == wantVisible) break;
      }
    });
    _uiSessionPumpFuture = pump.catchError((_) {});
  }

  Future<void> _applyUiSession(bool uiVisible, String? roomPublicKeyHex) async {
    if (!GhalBolP2p.isAvailable) return;
    if (GhalBolP2p.usesDaemon && !await GhalBolDaemonClient.probeDaemon()) {
      if (!uiVisible && roomPublicKeyHex == null) return;
      await GhalBolDaemonClient.ensureDaemonRunning();
    }
    if (!await GhalBolP2p.isRunning()) return;
    final r = await GhalBolP2p.syncUiSession(
      uiVisible: uiVisible,
      roomPublicKeyHex: roomPublicKeyHex,
    );
    if (r["ok"] != true) {
      P2pFlowLog.issue("sync_ui_session_failed", detail: r["error"]?.toString());
    } else {
      SessionFlowLog.step("ui_session_applied", {
        "visible": uiVisible.toString(),
        "room": roomPublicKeyHex != null && isValidPublicKeyHex(roomPublicKeyHex)
            ? P2pFlowLog.shortPk(roomPublicKeyHex)
            : "(none)",
        "read": r["read_receipts"]?.toString() ?? "?",
      });
    }
  }

  /// Starts poll loop immediately; P2P bootstrap runs in the background.
  Future<void> ensureStarted(GhalBolIdentityResult session) async {
    GhalBolP2p.pollEventDispatcher = dispatchPolledEvent;
    _appNs = session.appNamespace?.trim().isNotEmpty == true
        ? session.appNamespace!.trim()
        : kGhalBolAppNamespace;
    final firstStart = _poll == null;
    if (firstStart) {
      SessionFlowLog.step("bridge_start", {
        "ns": _appNs,
        "daemon": GhalBolP2p.usesDaemon.toString(),
      });
      AppLog.instance.trace("session_start", "poll bridge + network bootstrap");
    }
    _poll ??= Timer.periodic(const Duration(milliseconds: 200), (_) {
      unawaited(_drain());
    });
    _heartbeat ??= Timer.periodic(const Duration(minutes: 2), (_) {
      unawaited(_logSessionHeartbeat());
    });
    _startLinuxWakePollIfNeeded();
    _uiVisibleDesired = true;
    _scheduleUiSessionApply();
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

  /// Re-run hub UI session after P2P comes up (unlock is not blocked on this).
  void _reapplyDeferredSessionRpc() {
    unawaited(_scheduleUiSessionApplyAndWait());
  }

  Future<void> _scheduleUiSessionApplyAndWait() async {
    _scheduleUiSessionApply();
    await awaitUiSessionApplied();
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

  /// Process supervision only — NOT reconnect policy. The shell is the only thing that can relaunch
  /// a dead `ghal_bol_core_daemon` / Android `:p2p` process; Rust cannot restart its own host process.
  /// All connectivity policy (WAN recovery, coord lookup/register, dial, LAN handover, backoff)
  /// lives in `ghal_bol` (`chat_server` / `coord_runtime`). This only detects "node process not
  /// running", relaunches + unlocks it, and re-signals contacts (`syncContacts`). Keep it free of
  /// any dial/lookup/ack/transcript logic — see AGENTS.md SSOT split.
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
        if (!await GhalBolDaemonClient.probeDaemon(force: true)) {
          await GhalBolDaemonClient.reconnectDaemon();
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
          _reapplyDeferredSessionRpc();
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

  /// Linux window hide / bridge stop — DESIGN close order (hub owns normal room leave).
  Future<void> notifyNativeUiExited() async {
    _uiVisibleDesired = false;
    _foregroundDesired = null;
    _scheduleUiSessionApply();
    await awaitUiSessionApplied();
  }

  /// GTK **close (X)** only (`delete-event` → hide). Minimize/unfocus do not call this.
  Future<void> onLinuxWindowClosedByUser() async {
    if (!kIsWeb && defaultTargetPlatform == TargetPlatform.linux) {
      if (_linuxWindowClosedByUser) return;
      _linuxWindowClosedByUser = true;
      SessionFlowLog.step("linux_window_closed_by_user");
      await notifyNativeUiExited();
      _notifyLinuxWindowCloseListeners(true);
    }
  }

  /// User reopened the app after **close (X)** — notification tap or launcher activate.
  void onLinuxWindowRestoredFromClose() {
    if (!kIsWeb &&
        defaultTargetPlatform == TargetPlatform.linux &&
        _linuxWindowClosedByUser) {
      _linuxWindowClosedByUser = false;
      SessionFlowLog.step("linux_window_restored_from_close");
      _uiVisibleDesired = true;
      _scheduleUiSessionApply();
      _notifyLinuxWindowCloseListeners(false);
    }
  }

  Future<void> stop() async {
    AppLog.instance.i("Session", "bridge stop");
    await notifyNativeUiExited();
    if (identical(GhalBolP2p.pollEventDispatcher, dispatchPolledEvent)) {
      GhalBolP2p.pollEventDispatcher = null;
    }
    _poll?.cancel();
    _poll = null;
    _heartbeat?.cancel();
    _heartbeat = null;
    _linuxWakePoll?.cancel();
    _linuxWakePoll = null;
    _coordDialDebounce?.cancel();
    _coordDialDebounce = null;
    _coordDialInFlight = null;
    _networkBootstrapOk = false;
    _bootstrapFuture = null;
    _foregroundDesired = null;
    _uiSessionPumpFuture = null;
    _uiVisibleDesired = false;
    _streamReadyPeers.clear();
    _identifiedPeersHotRegistered.clear();
    _listeners.clear();
    _nodeReady = false;
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
    final guarded = next.catchError((Object error, StackTrace stack) {
      AppLog.instance.w("Daemon", "event poll interrupted — reconnecting");
      unawaited(recoverP2pIfNeeded());
    });
    _drainChain = guarded;
    return guarded;
  }

  /// Linux only: daemon wake files (unlock after reboot, incoming-call notify tap).
  void startLinuxWakePollIfNeeded() {
    _startLinuxWakePollIfNeeded();
  }

  void _startLinuxWakePollIfNeeded() {
    if (_linuxWakePoll != null) return;
    if (kIsWeb || defaultTargetPlatform != TargetPlatform.linux) return;
    if (!GhalBolP2p.usesDaemon) return;
    _linuxWakePoll = Timer.periodic(const Duration(seconds: 2), (_) {
      unawaited(_maybeHandleUnlockWake());
      unawaited(_maybeHandleIncomingCallWake());
    });
  }

  Future<void> _maybeHandleUnlockWake() async {
    if (!GhalBolP2p.usesDaemon) return;
    if (kIsWeb || defaultTargetPlatform != TargetPlatform.linux) return;
    try {
      if (!await GhalBolP2p.takeUnlockWake()) return;
      SessionFlowLog.step("unlock_wake", {"source": "daemon_autostart"});
      // Present only — no GhalBolUiSession / onAppForeground (DESIGN.md: no session-sync on wake).
      await CallIncomingAlert.presentWindow();
    } catch (_) {}
  }

  Future<void> _maybeHandleIncomingCallWake() async {
    if (!GhalBolP2p.usesDaemon) return;
    if (kIsWeb || defaultTargetPlatform != TargetPlatform.linux) return;
    try {
      if (!await GhalBolP2p.takeIncomingCallWake()) return;
      SessionFlowLog.step("incoming_call_wake", {"source": "daemon_notify"});
      await CallIncomingAlert.presentWindow();
      CallController.instance.onAppForeground();
    } catch (_) {}
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
    if (kind == "dm_message") {
      final msgKind = ev["msg_kind"]?.toString() ?? "";
      if (msgKind == "text") {
        // Native may have persisted on wire before this poll replay — still refresh roster badge.
        ContactStore.previewChangeCount.value++;
      }
    }
    if (ev["stores_updated"] == true) {
      AppLog.instance.flow("DM/store", "UI refresh: stores_updated kind=$kind");
      final msgKind = ev["msg_kind"]?.toString() ?? "";
      // DESIGN.md / AGENTS.md: roster bump on peer_identified only — not every inbound text
      // (each text was forcing sync_contacts → p2p_register_dm_peer storm on the main RPC socket).
      if (kind == "peer_identified") {
        ContactStore.rosterChangeCount.value++;
        _scheduleCoordDialIfNeeded();
      } else if (kind == "dm_message" && msgKind == "text") {
        // Already bumped above for every inbound text poll event.
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
      final pk = streamContactKeyFromEvent(ev);
      if (pk.isNotEmpty && _streamReadyPeers.add(pk)) {
        P2pFlowLog.step("stream_ready", {
          "peer": P2pFlowLog.shortPk(pk),
          "kind": kind ?? "?",
          "count": _streamReadyPeers.length.toString(),
        });
      }
    }
    if (kind == "peer_disconnected") {
      final pk = streamContactKeyFromEvent(ev);
      if (pk.isNotEmpty && _streamReadyPeers.remove(pk)) {
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
    final inFlight = _coordDialInFlight;
    if (inFlight != null) {
      return inFlight;
    }
    Future<void> run() async {
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

    final next = run();
    _coordDialInFlight = next;
    try {
      await next;
    } finally {
      if (identical(_coordDialInFlight, next)) {
        _coordDialInFlight = null;
      }
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
