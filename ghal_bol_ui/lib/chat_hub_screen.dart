import "dart:async" show Timer, unawaited;
import "dart:io";

import "package:flutter/foundation.dart" show kIsWeb;
import "package:flutter/material.dart";
import "package:flutter/services.dart";
import "app_log.dart";
import "app_log_screen.dart";
import "blocked_peers_screen.dart";
import "chat_screen.dart";
import "chat_transcript_store.dart";
import "contact_store.dart";
import "contacts_screen.dart";
import "embedder_storage.dart";
import "ghal_bol_constants.dart";
import "native_build_hint.dart";
import "ghal_bol_ffi.dart";
import "ghal_bol_background.dart";
import "ghalbol_connect_invite.dart";
import "identity_alias_form.dart";
import "identity_key_management.dart";
import "invite_scan_screen.dart";
import "package:share_plus/share_plus.dart";
import "ghal_bol_ui_session.dart";
import "p2p_event_bridge.dart";
import "p2p_network_coordinator.dart";
import "saved_contact.dart";
import "share_invite_screen.dart";

import "invite_uri_builder.dart";
import "identity_display_name.dart";
import "public_key_hex.dart";
import "identity_alias_store.dart";
import "responsive_breakpoints.dart";
import "chat_wallpaper.dart";
import "hub_back_stack.dart";
import "hub_roster_selection.dart";
import "call/call_controller.dart";
import "ghal_bol_p2p.dart";
import "invite_deep_link.dart";

/// Post-unlock shell: responsive **Chats / Identity / More** chrome (bottom bar on narrow
/// windows, [NavigationRail] on wide), plus chat list + room.
class ChatHubScreen extends StatefulWidget {
  const ChatHubScreen({
    super.key,
    required this.session,
    required this.onUiLock,
    required this.onEndSession,
  });

  final GhalBolIdentityResult session;
  /// UI-only: hide hub; P2P and poll keep running.
  final VoidCallback onUiLock;
  /// Logout / delete identity: tear down session.
  final VoidCallback onEndSession;

  @override
  State<ChatHubScreen> createState() => ChatHubScreenState();
}

class ChatHubScreenState extends State<ChatHubScreen> with WidgetsBindingObserver {
  /// Shell tab: 0 = Chats, 1 = Identity, 2 = More.
  int _navTab = 0;

  /// Narrow layout only: `false` = list, `true` = room (IndexedStack index).
  bool _narrowShowRoom = false;

  /// Wide/split: `true` while the chat column is the active conversation (see [_syncNativeForegroundPeer]).
  bool _splitChatEngaged = false;

  final HubHistoryStack _hubHistory = HubHistoryStack();

  /// Active hub chat surface (no [GlobalKey] — avoids duplicate-key crashes).
  ChatScreenState? _attachedHubChat;

  GhalBolIdentityResult get _s => widget.session;

  String get _appNs => _s.appNamespace ?? kGhalBolAppNamespace;

  /// Custom display name from [IdentityAliasStore]; `null` means use signing-key hex default.
  String? _storedCustomAlias;
  int _aliasSaveNonce = 0;

  List<SavedContact> _contacts = [];
  /// Selected roster row — [SavedContact.conversationKey] (`public_key_hex`).
  String? _selectedConversationKey;
  /// Contact from the last [_selectContact] tap — stable before roster reload finds the row.
  SavedContact? _openRoomContact;
  String _searchQuery = "";
  Timer? _rosterSyncDebounce;
  Timer? _contactsPreviewDebounce;

  SavedContact? get _selectedContact {
    final key = _selectedConversationKey;
    if (key == null || key.isEmpty) return null;
    final open = _openRoomContact;
    if (open != null && open.conversationKey == key) return open;
    for (final c in _contacts) {
      if (c.conversationKey == key) return c;
    }
    return null;
  }

  bool _hubShellSplit(BuildContext context) => ghalBolUseChatShellSplit(context);

  HubHistoryEntry _hubHistorySnapshot() => HubHistoryEntry(
        navTab: _navTab,
        narrowShowRoom: _narrowShowRoom,
        splitChatEngaged: _splitChatEngaged,
        conversationKey: _selectedConversationKey,
      );

  void _recordHubNavigation() {
    _hubHistory.recordNavigate(_hubHistorySnapshot());
  }

  void _applyHubHistoryEntry(HubHistoryEntry entry) {
    setState(() {
      _navTab = entry.navTab;
      _narrowShowRoom = entry.narrowShowRoom;
      _splitChatEngaged = entry.splitChatEngaged;
      final incomingKey = entry.conversationKey?.trim();
      if (incomingKey != null && incomingKey.isNotEmpty) {
        _selectedConversationKey = incomingKey;
        if (_openRoomContact?.conversationKey != incomingKey) {
          _openRoomContact = _selectedContact;
        }
      } else if (!entry.narrowShowRoom && !entry.splitChatEngaged && _navTab == 0) {
        // Roster-only back — keep stable thread key for mounted [ChatScreen] (DESIGN.md hubThreadKey).
      } else {
        _selectedConversationKey = null;
        _openRoomContact = null;
      }
    });
    _syncNativeForegroundPeer();
  }

  /// Hub chrome still above root (room open or non-Chats tab) — used if history desyncs.
  bool _hubHasChromeToUnwind() =>
      _isHubChatRoomOpen(context) || _navTab != 0;

  /// One step back without relying on history (same UI + foreground contract as before).
  void _hubUnwindChromeFallback() {
    final split = _hubShellSplit(context);
    if (_isHubChatRoomOpen(context)) {
      setState(() {
        if (split) {
          _splitChatEngaged = false;
        } else {
          _narrowShowRoom = false;
        }
      });
    } else if (_navTab != 0) {
      setState(() => _navTab = 0);
    }
    _hubHistory.replaceTop(_hubHistorySnapshot());
    _syncNativeForegroundPeer();
  }

  /// System back (Android edge gesture, desktop mouse back). Called from [GhalBolRoot].
  ///
  /// Returns `true` when this back press was handled; `false` at hub root so the shell
  /// may exit the app. Always use with a parent [PopScope] where `canPop` is `false`.
  ///
  /// Pushed [Navigator] routes pop first. Hub history pops next and always runs
  /// [_syncNativeForegroundPeer] (leave: clear foreground then disable read gate).
  bool handleHubSystemBack() {
    final nav = Navigator.of(context);
    if (nav.canPop()) {
      nav.pop();
      return true;
    }
    final prev = _hubHistory.pop();
    if (prev == null) {
      if (_hubHasChromeToUnwind()) {
        _hubUnwindChromeFallback();
        return true;
      }
      return false;
    }
    _applyHubHistoryEntry(prev);
    return true;
  }

  void _popHubChromeBack() {
    if (!handleHubSystemBack()) {
      SystemNavigator.pop();
    }
  }

  /// Native `ack_read` only while the chat **room** is visible (DESIGN.md).
  int _foregroundSyncEpoch = 0;

  /// Read acks only while the chat room is on screen — not after GTK close (X).
  /// Minimize and unfocus do not set [linuxWindowClosedByUser].
  bool _nativeForegroundRoomOpen(BuildContext context) {
    if (!_isHubChatRoomOpen(context)) return false;
    if (!kIsWeb &&
        Platform.isLinux &&
        P2pEventBridge.instance.linuxWindowClosedByUser) {
      return false;
    }
    return true;
  }

  void _onLinuxWindowCloseChanged(bool closedByUser) {
    if (closedByUser) {
      _foregroundSyncEpoch++;
      _layoutSyncedRoomOpen = false;
    }
    _syncNativeForegroundPeer();
  }

  /// Last [_isHubChatRoomOpen] we pushed to native — detect resize list-only without a tap.
  bool? _layoutSyncedRoomOpen;
  String? _lastSyncedForegroundPk;
  bool _layoutSyncPostFrameScheduled = false;
  Timer? _layoutCloseDebounce;
  Timer? _roomOpenConfirmTimer;
  Timer? _readGateNudgeDebounce;
  Timer? _readGateKeepaliveTimer;
  int _lastReadGateNudgeMs = 0;
  static const _readGateNudgeMinGapMs = 1500;

  void _syncNativeForegroundPeer() {
    unawaited(_syncNativeForegroundPeerAsync());
  }

  /// Stable pk for native foreground — not [_selectedContact] (roster reload null frame).
  String? _nativeForegroundPublicKey() {
    final key = _selectedConversationKey?.trim().toLowerCase() ?? "";
    if (isValidPublicKeyHex(key)) return key;
    final c = _selectedContact;
    if (c == null) return null;
    final pk = c.publicKeyHex.trim().toLowerCase();
    return isValidPublicKeyHex(pk) ? pk : null;
  }

  /// True when the user is **seeing** the conversation UI (not a selected row in the list).
  ///
  /// Desktop split: the right pane always shows [_chatBody] for the selected row — native read
  /// gate must follow [_selectedConversationKey], not [_splitChatEngaged] (history/back can
  /// clear engaged while the thread stays on screen; that was recv-only + spurious leave drain).
  /// Narrow shell: list-only until [_narrowShowRoom] (back from chat).
  bool _isHubChatRoomOpen(BuildContext context) {
    if (_navTab != 0) return false;
    // DESIGN.md hubThreadKey — gate on stable key, not roster row lookup.
    final key = _selectedConversationKey;
    if (key == null || key.isEmpty || !isValidPublicKeyHex(key)) return false;
    if (ghalBolUseChatShellSplit(context)) {
      return true;
    }
    return _narrowShowRoom;
  }

  /// Window crossed split breakpoint or room visibility changed — native must match UI.
  /// Post-frame only: MediaQuery can flicker one frame on resize and must not close read gate.
  void _syncNativeForegroundIfLayoutChanged(BuildContext context) {
    if (_layoutSyncPostFrameScheduled) return;
    _layoutSyncPostFrameScheduled = true;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      _layoutSyncPostFrameScheduled = false;
      if (!mounted) return;
      final nowSplit = ghalBolUseChatShellSplit(context);
      // Wide → narrow: list-only; engaged flag is stale until user taps the chat pane again.
      if (!nowSplit && _splitChatEngaged) {
        setState(() => _splitChatEngaged = false);
        _hubHistory.replaceTop(_hubHistorySnapshot());
      }
      final roomNow = _isHubChatRoomOpen(context);
      if (_layoutSyncedRoomOpen == roomNow) return;
      if (!roomNow) {
        // Debounce room-close on resize flicker — must not run leave drain spuriously.
        _layoutCloseDebounce?.cancel();
        _layoutCloseDebounce = Timer(const Duration(milliseconds: 120), () {
          if (!mounted) return;
          if (_isHubChatRoomOpen(context)) return;
          if (_layoutSyncedRoomOpen == false) return;
          _syncNativeForegroundPeer();
        });
        return;
      }
      _layoutCloseDebounce?.cancel();
      _syncNativeForegroundPeer();
    });
  }

  bool _openRoomMatchesPeerKey(String pk) {
    if (pk.isEmpty) return false;
    final sel = _selectedConversationKey?.trim().toLowerCase() ?? "";
    if (sel.isNotEmpty && publicKeysEqual(sel, pk)) return true;
    final fg = _nativeForegroundPublicKey();
    return fg != null && publicKeysEqual(fg, pk);
  }

  /// Linux desktop: `:p2p` read gate can drift while chat pane stays visible.
  /// Re-run `p2p_sync_ui_session` (debounced) — same effect as resize, without layout churn.
  void _scheduleReadGateNudge({String? reason}) {
    if (kIsWeb || !Platform.isLinux) return;
    if (!_nativeForegroundRoomOpen(context)) return;
    _readGateNudgeDebounce?.cancel();
    _readGateNudgeDebounce = Timer(const Duration(milliseconds: 250), () {
      if (!mounted || !_nativeForegroundRoomOpen(context)) return;
      final now = DateTime.now().millisecondsSinceEpoch;
      if (now - _lastReadGateNudgeMs < _readGateNudgeMinGapMs) return;
      _lastReadGateNudgeMs = now;
      AppLog.instance.flow(
        "Hub",
        "read gate nudge${reason != null ? " ($reason)" : ""}",
      );
      GhalBolUiSession.setVisible(true);
      GhalBolUiSession.nudge();
    });
  }

  void _syncReadGateKeepalive(bool roomOpen) {
    if (kIsWeb || !Platform.isLinux || !roomOpen) {
      _readGateKeepaliveTimer?.cancel();
      _readGateKeepaliveTimer = null;
      return;
    }
    if (_readGateKeepaliveTimer != null) return;
    _readGateKeepaliveTimer = Timer.periodic(const Duration(seconds: 8), (_) {
      _scheduleReadGateNudge(reason: "keepalive");
    });
  }

  /// One deferred re-push of the open room — covers native miss before P2P/daemon ready.
  /// Single shot per room-open epoch; not a per-frame retry (DESIGN.md forbidden patch).
  void _scheduleRoomOpenConfirmSync(int epoch) {
    _roomOpenConfirmTimer?.cancel();
    _roomOpenConfirmTimer = Timer(const Duration(milliseconds: 400), () {
      if (!mounted || epoch != _foregroundSyncEpoch) return;
      if (!_nativeForegroundRoomOpen(context)) return;
      final pk = _nativeForegroundPublicKey();
      if (pk == null) return;
      GhalBolUiSession.setVisible(true);
      GhalBolUiSession.setRoom(pk);
      unawaited(GhalBolUiSession.awaitApplied().then((_) {
        if (mounted && epoch == _foregroundSyncEpoch) {
          _layoutSyncedRoomOpen = true;
          _lastSyncedForegroundPk = pk;
        }
      }));
    });
  }

  Future<void> _syncNativeForegroundPeerAsync() async {
    final epoch = ++_foregroundSyncEpoch;
    _roomOpenConfirmTimer?.cancel();
    final roomOpen = _nativeForegroundRoomOpen(context);
    if (!roomOpen) {
      if (_layoutSyncedRoomOpen == false && _lastSyncedForegroundPk == null) {
        return;
      }
      AppLog.instance.flow(
        "Hub",
        "room closed → sync ui session (no room, leave drain)",
      );
      _lastSyncedForegroundPk = null;
      GhalBolUiSession.setRoom(null);
      await GhalBolUiSession.awaitApplied();
      if (mounted && epoch == _foregroundSyncEpoch) {
        _layoutSyncedRoomOpen = false;
        _syncReadGateKeepalive(false);
      }
      return;
    }
    final pk = _nativeForegroundPublicKey();
    if (pk == null) return;
    AppLog.instance.flow(
      "Hub",
      "room open → sync ui session pk=${pk.length > 16 ? "${pk.substring(0, 8)}…" : pk}",
    );
    final c = _selectedContact;
    if (c != null) {
      unawaited(ContactStore.clearUnreadForContact(appNamespace: _appNs, contact: c));
    }
    GhalBolUiSession.setVisible(true);
    GhalBolUiSession.setRoom(pk);
    await GhalBolUiSession.awaitApplied();
    if (mounted && epoch == _foregroundSyncEpoch) {
      _layoutSyncedRoomOpen = true;
      _lastSyncedForegroundPk = pk;
      _scheduleRoomOpenConfirmSync(epoch);
      _syncReadGateKeepalive(true);
    }
  }

  List<SavedContact> get _filteredContacts {
    final q = _searchQuery.trim().toLowerCase();
    if (q.isEmpty) return _contacts;
    return _contacts.where((c) {
      final label = ghalBolIdName(
        publicKeyHex: c.publicKeyHex,
        customAlias: c.displayAlias,
      ).toLowerCase();
      return label.contains(q) || c.publicKeyHex.toLowerCase().contains(q);
    }).toList();
  }

  late final VoidCallback _onCallEndedReloadChat;

  @override
  void initState() {
    super.initState();
    _onCallEndedReloadChat = () {
      _attachedHubChat?.onHubReattached(reloadTranscript: true);
      _syncNativeForegroundPeer();
    };
    CallController.addCallEndedListener(_onCallEndedReloadChat);
    _hubHistory.reset(_hubHistorySnapshot());
    WidgetsBinding.instance.addObserver(this);
    _loadStoredAlias();
    ContactStore.rosterChangeCount.addListener(_onRosterChanged);
    ContactStore.changeCount.addListener(_onContactsUiChanged);
    ContactStore.previewChangeCount.addListener(_onPreviewPollChanged);
    P2pEventBridge.instance.addListener(_routeHubP2pEvent);
    P2pEventBridge.instance.addLinuxWindowCloseListener(_onLinuxWindowCloseChanged);
    InviteDeepLink.onInviteUri = (uri) {
      if (mounted) unawaited(_joinFromUri(uri));
    };
    unawaited(_bootstrapHub());
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    switch (state) {
      case AppLifecycleState.resumed:
        AppLog.instance.flow("Hub", "lifecycle resumed");
        GhalBolUiSession.setVisible(true);
        unawaited(GhalBolBackground.onAppResumed());
        _syncNativeForegroundPeer();
        _attachedHubChat?.onHubReattached(reloadTranscript: true);
        P2pEventBridge.instance.drainNow();
      case AppLifecycleState.inactive:
        // Android: shade, task switcher — stop new read receipts. Linux: window drag /
        // brief focus loss during resize must not clear read gate (room stays on screen).
        if (!kIsWeb && Platform.isLinux) {
          AppLog.instance.flow(
            "Hub",
            "lifecycle inactive on Linux — read gate unchanged (room on screen)",
          );
          break;
        }
        AppLog.instance.flow("Hub", "lifecycle inactive → ui not visible (room unchanged)");
        GhalBolUiSession.setVisible(false);
      case AppLifecycleState.paused:
      case AppLifecycleState.hidden:
      case AppLifecycleState.detached:
        unawaited(_onAppBackgrounded(state));
    }
  }

  Future<void> _onAppBackgrounded(AppLifecycleState state) async {
    // Linux desktop: paused/hidden can mean minimize — not "left chat". Close (X) is separate.
    if (!kIsWeb && Platform.isLinux) {
      AppLog.instance.flow(
        "Hub",
        "lifecycle $state on Linux — read acks unchanged (close X clears room)",
      );
      return;
    }
    AppLog.instance.flow(
      "Hub",
      "lifecycle $state → close room + ui session off",
    );
    GhalBolUiSession.setVisible(false);
    GhalBolUiSession.setRoom(null);
    await GhalBolUiSession.awaitApplied();
  }

  @override
  void dispose() {
    CallController.removeCallEndedListener(_onCallEndedReloadChat);
    if (InviteDeepLink.onInviteUri != null) {
      InviteDeepLink.onInviteUri = null;
    }
    P2pEventBridge.instance.removeListener(_routeHubP2pEvent);
    P2pEventBridge.instance.removeLinuxWindowCloseListener(_onLinuxWindowCloseChanged);
    GhalBolUiSession.setRoom(null);
    unawaited(() async {
      await GhalBolUiSession.awaitApplied();
      GhalBolUiSession.setVisible(false);
      await GhalBolUiSession.awaitApplied();
    }());
    WidgetsBinding.instance.removeObserver(this);
    _rosterSyncDebounce?.cancel();
    _contactsPreviewDebounce?.cancel();
    _layoutCloseDebounce?.cancel();
    _roomOpenConfirmTimer?.cancel();
    _readGateNudgeDebounce?.cancel();
    _readGateKeepaliveTimer?.cancel();
    ContactStore.rosterChangeCount.removeListener(_onRosterChanged);
    ContactStore.changeCount.removeListener(_onContactsUiChanged);
    ContactStore.previewChangeCount.removeListener(_onPreviewPollChanged);
    super.dispose();
  }

  void _onRosterChanged() {
    _rosterSyncDebounce?.cancel();
    _rosterSyncDebounce = Timer(const Duration(milliseconds: 600), () {
      unawaited(_reloadContactsAndSyncP2pIfNeeded());
    });
  }

  void _onContactsUiChanged() {
    _contactsPreviewDebounce?.cancel();
    _contactsPreviewDebounce = Timer(const Duration(milliseconds: 400), () {
      unawaited(_reloadContactsListOnly());
    });
  }

  void _onPreviewPollChanged() {
    _scheduleReadGateNudge(reason: "preview");
    _contactsPreviewDebounce?.cancel();
    _contactsPreviewDebounce = Timer(const Duration(milliseconds: 400), () {
      unawaited(_reloadContactsListOnly());
    });
  }

  /// Load roster, then start P2P in the background (do not block unlock UI).
  Future<void> _bootstrapHub() async {
    await _reloadContactsListOnly();
    if (!mounted) return;
    unawaited(P2pEventBridge.instance.ensureStarted(_s));
    _syncNativeForegroundPeer();
    final pending = InviteDeepLink.takePending();
    if (pending != null && mounted) {
      await _joinFromUri(pending);
    }
    unawaited(_checkUnusedAppRestrictions());
    unawaited(cancelUnlockNotification());
  }

  static bool _backgroundCheckDone = false;

  Future<void> _checkUnusedAppRestrictions() async {
    if (_backgroundCheckDone) return;
    _backgroundCheckDone = true;
    if (!Platform.isAndroid) return;
    final enabled = await isUnusedAppPauseEnabled();
    if (!enabled || !mounted) return;
    AppLog.instance.flow("Hub", "unused app pause enabled — showing prompt");
    showDialog<void>(
      context: context,
      builder: (ctx) => AlertDialog(
        title: const Text("Background messaging restricted"),
        content: const Text(
          '"Pause app activity if unused" is enabled for Ghal Bol. '
          "This prevents the app from receiving messages when the screen is off.\n\n"
          "Please disable it in the next screen to ensure reliable message delivery.",
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(ctx).pop(),
            child: const Text("Later"),
          ),
          FilledButton(
            onPressed: () {
              Navigator.of(ctx).pop();
              openUnusedAppSettings();
            },
            child: const Text("Open Settings"),
          ),
        ],
      ),
    );
  }

  Future<void> _reloadContactsListOnly() async {
    final list = await ContactStore.listContacts(_appNs);
    if (!mounted) return;
    setState(() {
      _contacts = list;
      _selectedConversationKey = preserveHubConversationSelection(
        selectedConversationKey: _selectedConversationKey,
        rosterKeys: list.map((c) => c.conversationKey).toList(),
      );
    });
  }

  String? _computeInviteUri() {
    final pk = _s.publicKeyHex?.trim() ?? "";
    if (!isValidPublicKeyHex(pk)) return null;
    return buildGhalBolInviteUri(
      publicKeyHex: pk,
      peerAlias: _storedCustomAlias,
    );
  }

  Future<void> _showMyQrInvitation() async {
    if (!GhalBolFfi.isP2pAvailable) return;
    await _loadStoredAlias();
    P2pEventBridge.instance.drainNow();
    if (!mounted) return;
    final pk = _s.publicKeyHex?.trim() ?? "";
    if (!isValidPublicKeyHex(pk)) return;
    await Navigator.of(context).push<void>(
      MaterialPageRoute(
        builder: (_) => ShareInviteScreen(
          publicKeyHex: pk,
          appNamespace: _appNs,
          readListenReady: () => P2pEventBridge.instance.isNodeReady,
          onParentRefresh: () {
            if (mounted) {
              unawaited(_loadStoredAlias());
            }
          },
        ),
      ),
    );
  }

  Future<void> _openJoinScan() async {
    final st = _attachedHubChat;
    if (st != null) {
      await st.requestJoinFlow();
      return;
    }
    String? got;
    if (kIsWeb) {
      final data = await Clipboard.getData(Clipboard.kTextPlain);
      got = data?.text?.trim();
    } else if (Platform.isAndroid || Platform.isIOS) {
      final mode = await showModalBottomSheet<String>(
        context: context,
        showDragHandle: true,
        builder: (ctx) => SafeArea(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              ListTile(
                leading: const Icon(Icons.qr_code_scanner),
                title: const Text("Scan QR code"),
                onTap: () => Navigator.pop(ctx, "scan"),
              ),
              ListTile(
                leading: const Icon(Icons.link),
                title: const Text("Paste invitation link"),
                onTap: () => Navigator.pop(ctx, "paste"),
              ),
            ],
          ),
        ),
      );
      if (!mounted || mode == null) return;
      if (mode == "paste") {
        await _pasteAndJoin();
        return;
      }
      got = await Navigator.of(context).push<String>(
        MaterialPageRoute(builder: (_) => const InviteScanScreen()),
      );
    } else {
      await _pasteAndJoin();
      return;
    }
    if (!mounted || got == null || got.isEmpty) return;
    final uri = InviteScanScreen.extractInviteUri(got) ?? got.trim();
    await _joinFromUri(uri);
  }

  /// Reload roster from disk; call [P2pNetworkCoordinator.syncContacts] only when dm_peers set changed.
  Future<void> _reloadContactsAndSyncP2pIfNeeded({
    bool awaitP2p = false,
    bool forceSync = false,
  }) async {
    final fpBefore = P2pNetworkCoordinator.dmPeersFingerprint(_contacts);
    final countBefore = _contacts.length;
    await _reloadContactsListOnly();
    if (!mounted) return;
    final fpAfter = P2pNetworkCoordinator.dmPeersFingerprint(_contacts);
    final countAfter = _contacts.length;
    final needsSync = forceSync || fpBefore != fpAfter || countBefore != countAfter;
    AppLog.instance.flow(
      "Hub",
      "roster reload count=$countBefore→$countAfter needsSync=$needsSync force=$forceSync",
    );
    if (!needsSync) return;
    final sync = P2pNetworkCoordinator.syncContacts(
      _contacts,
      appNamespace: _appNs,
    );
    if (awaitP2p) {
      await sync;
    } else {
      unawaited(sync);
    }
  }

  Future<void> _reloadContactsAndSyncP2p({bool awaitP2p = false}) =>
      _reloadContactsAndSyncP2pIfNeeded(awaitP2p: awaitP2p, forceSync: true);

  Future<SavedContact?> _onContactJoined(SavedContact contact) async {
    final saved = await ContactStore.upsertContact(appNamespace: _appNs, contact: contact);
    if (!mounted) return null;
    _openRoomContact = saved;
    setState(() => _selectedConversationKey = saved.conversationKey);
    _syncNativeForegroundPeer();
    await _reloadContactsAndSyncP2p(awaitP2p: true);
    P2pEventBridge.instance.drainNow();
    if (!mounted) return null;
    if (!saved.hasFullKeys) {
      ScaffoldMessenger.of(context).showSnackBar(
        const SnackBar(
          content: Text(
            "Contact saved but keys are incomplete — try scanning the QR again.",
          ),
        ),
      );
    }
    return saved;
  }

  Future<void> _selectContact(SavedContact c) async {
    _openRoomContact = c;
    setState(() {
      _selectedConversationKey = c.conversationKey;
      if (!ghalBolUseChatShellSplit(context)) {
        _narrowShowRoom = true;
      } else {
        // Split: selecting a row opens the room in the right pane (same as Android narrow).
        _splitChatEngaged = true;
      }
    });
    _recordHubNavigation();
    _syncNativeForegroundPeer();
    P2pEventBridge.instance.drainNow();
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted) return;
      _attachedHubChat?.onHubReattached();
    });
    unawaited(ContactStore.clearUnreadForContact(appNamespace: _appNs, contact: c));
    unawaited(
      ChatTranscriptStore.warmThreadCache(
        appNamespace: _appNs,
        conversationKey: c.conversationKey,
        conversationKeys: c.allConversationKeys,
      ),
    );
    if (c.hasFullKeys) {
      unawaited(P2pNetworkCoordinator.registerContacts([c]));
      unawaited(
        P2pNetworkCoordinator.refreshCoordDial(
          [c],
          appNamespace: _appNs,
        ),
      );
    }
    // New [ChatScreen] mounts via [ValueKey] — [onHubChatAttach] reloads the right thread.
  }

  void _routeHubP2pEvent(Map<String, dynamic> ev) {
    if (!mounted) return;
    final kind = ev["kind"]?.toString();
    if (kind == "peer_identified") {
      final pk = contactPublicKeyHexFromEvent(ev);
      if (isValidPublicKeyHex(pk)) {
        unawaited(_onPeerKeysLearned(pk));
      }
    }
    if (kind == "peer_disconnected") {
      final pk = contactKeyFromEvent(ev);
      if (pk.isEmpty) return;
      final sel = _selectedContact?.publicKeyHex.trim() ?? "";
      if (sel.isNotEmpty && publicKeysEqual(sel, pk)) {
        AppLog.instance.flow("Hub", "selected peer disconnected pk=${pk.substring(0, 8)}…");
        _attachedHubChat?.onHubPeerLinkLost();
      }
    }
    if (kind == "dm_message") {
      final msgKind = ev["msg_kind"]?.toString() ?? "";
      if (msgKind == "text") {
        final pk = contactKeyFromEvent(ev);
        if (_openRoomMatchesPeerKey(pk)) {
          _scheduleReadGateNudge(reason: "inbound_text");
          if (ev["stores_updated"] == true) {
            setState(() {});
          }
        }
      }
    }
    if (kind == "chat_ready" || kind == "peer_connected") {
      final pk = streamContactKeyFromEvent(ev);
      if (_openRoomMatchesPeerKey(pk)) {
        _scheduleReadGateNudge(reason: kind ?? "stream");
      }
    }
    if (kind == "node_ready") {
      setState(() {});
    }
    _attachedHubChat?.ingestP2pEvent(ev);
  }

  Future<void> _onPeerKeysLearned(String publicKeyHex) async {
    await ContactStore.mergeDiscoveredContact(
      appNamespace: _appNs,
      publicKeyHex: publicKeyHex,
    );
    if (!mounted) return;
    await _reloadContactsListOnly();
    SavedContact? contact;
    for (final c in _contacts) {
      if (publicKeysEqual(c.publicKeyHex, publicKeyHex)) {
        contact = c;
        break;
      }
    }
    if (contact != null && contact.hasFullKeys) {
      unawaited(P2pNetworkCoordinator.registerContacts([contact]));
    }
    _attachedHubChat?.ingestP2pEvent({
      "kind": "peer_identified",
      "public_key_hex": publicKeyHex,
    });
  }

  void _attachHubChat(ChatScreenState state) {
    _attachedHubChat = state;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted || _attachedHubChat != state) return;
      state.onHubReattached();
    });
  }

  void _detachHubChat(ChatScreenState state) {
    if (_attachedHubChat == state) _attachedHubChat = null;
  }

  bool _hubPeerStreamReady(String publicKeyHex) =>
      P2pEventBridge.instance.isStreamReady(publicKeyHex);

  Future<void> _openNewChatMenu() async {
    final action = await showModalBottomSheet<String>(
      context: context,
      showDragHandle: true,
      builder: (ctx) => SafeArea(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            ListTile(
              leading: const Icon(Icons.qr_code_scanner),
              title: const Text("Scan invitation QR"),
              onTap: () => Navigator.pop(ctx, "scan"),
            ),
            ListTile(
              leading: const Icon(Icons.link),
              title: const Text("Paste invitation link"),
              onTap: () => Navigator.pop(ctx, "paste"),
            ),
            ListTile(
              leading: const Icon(Icons.person_add_outlined),
              title: const Text("Add contact by public keys"),
              onTap: () => Navigator.pop(ctx, "keys"),
            ),
          ],
        ),
      ),
    );
    if (!mounted || action == null) return;
    if (action == "scan") {
      final uri = await Navigator.of(context).push<String>(
        MaterialPageRoute(builder: (_) => const InviteScanScreen()),
      );
      if (uri != null && mounted) await _joinFromUri(uri);
    } else if (action == "paste") {
      await _pasteAndJoin();
    } else if (action == "keys") {
      await Navigator.of(context).push(
        MaterialPageRoute(builder: (_) => ContactsScreen(appNamespace: _appNs)),
      );
    }
  }

  Future<void> _pasteAndJoin() async {
    final data = await Clipboard.getData(Clipboard.kTextPlain);
    final text = data?.text?.trim();
    if (text == null || text.isEmpty) {
      if (!mounted) return;
      ScaffoldMessenger.of(context).showSnackBar(
        const SnackBar(content: Text("Clipboard is empty.")),
      );
      return;
    }
    await _joinFromUri(text);
  }

  Future<void> _joinFromUri(String uri) async {
    if (!GhalBolFfi.isP2pAvailable) {
      if (!mounted) return;
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(
          content: Text(
            "Adding a contact needs the native library. ${NativeBuildHint.rebuildInstructions}",
          ),
        ),
      );
      return;
    }
    final inv = GhalBolConnectInvite.tryParseInviteUri(uri);
    if (inv == null) {
      if (!mounted) return;
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(
          content: Text(
            GhalBolConnectInvite.explainInviteParseFailure(uri) ??
                "Not a valid Ghal Bol invitation.",
          ),
        ),
      );
      return;
    }
    AppLog.instance.i("Invite", "scan ok public_key_hex=${inv.publicKeyHex.substring(0, 8)}…");
    final saved = await _onContactJoined(SavedContact.fromInvite(inv));
    if (!mounted || saved == null) return;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted) return;
      setState(() {
        _navTab = 0;
        _narrowShowRoom = true;
        if (ghalBolUseChatShellSplit(context)) {
          _splitChatEngaged = true;
        }
      });
      _recordHubNavigation();
      _syncNativeForegroundPeer();
      if (saved.hasFullKeys) {
        unawaited(P2pNetworkCoordinator.registerContacts([saved]));
        unawaited(
          P2pNetworkCoordinator.refreshCoordDial(
            [saved],
            appNamespace: _appNs,
          ),
        );
      }
      P2pEventBridge.instance.drainNow();
    });
  }

  Future<void> _loadStoredAlias() async {
    final pk = _s.publicKeyHex?.trim();
    if (!isValidPublicKeyHex(pk)) return;
    final v = await IdentityAliasStore.read(
      appNamespace: _s.appNamespace ?? kGhalBolAppNamespace,
      publicKeyHex: pk!,
    );
    if (!mounted) return;
    setState(() => _storedCustomAlias = v);
  }

  Future<void> _copyPublicKey(BuildContext context) async {
    final pk = _s.publicKeyHex?.trim() ?? "";
    if (!isValidPublicKeyHex(pk)) return;
    await Clipboard.setData(ClipboardData(text: pk));
    if (!context.mounted) return;
    ScaffoldMessenger.of(context).showSnackBar(
      const SnackBar(content: Text("Public key copied (66 hex).")),
    );
  }

  Future<void> _copyInvitationFromHub(BuildContext context) async {
    final st = _attachedHubChat;
    if (st != null) {
      await st.requestCopyInvitationLink();
      return;
    }
    await _loadStoredAlias();
    final uri = _computeInviteUri();
    if (uri == null) return;
    await Clipboard.setData(ClipboardData(text: uri));
    if (!context.mounted) return;
    ScaffoldMessenger.of(context).showSnackBar(
      const SnackBar(content: Text("Invitation copied")),
    );
  }

  Future<void> _shareInvitationFromHub(BuildContext context) async {
    await _loadStoredAlias();
    final uri = _computeInviteUri();
    if (uri == null) return;
    await SharePlus.instance.share(
      ShareParams(text: uri, subject: "Ghal Bol invitation"),
    );
  }

  void _openChatInvitationFromHub() {
    if (!GhalBolFfi.isP2pAvailable) return;
    if (_navTab != 0) {
      setState(() => _navTab = 0);
      _recordHubNavigation();
    }
    unawaited(_showMyQrInvitation());
  }

  void _onHubNavTabSelected(int index) {
    if (index == _navTab) return;
    setState(() => _navTab = index);
    _recordHubNavigation();
    _syncNativeForegroundPeer();
  }

  String? _selectedContactCallPk() {
    final c = _selectedContact;
    if (c == null) return null;
    final pk = resolvePublicKeyHex(
      storedHex: c.publicKeyHex,
    );
    return isValidPublicKeyHex(pk) ? pk!.toLowerCase() : null;
  }

  String _selectedContactCallDisplayName() {
    final c = _selectedContact;
    if (c == null) return "Contact";
    return ghalBolIdName(
      publicKeyHex: c.publicKeyHex,
      customAlias: c.displayAlias,
    );
  }

  Widget _hubChatRoomPopupMenu(BuildContext context, GhalBolChatRoomPalette p) {
    final callPk = _selectedContactCallPk();
    final showCall = GhalBolP2p.isAvailable && callPk != null;
    return PopupMenuButton<String>(
      icon: Icon(Icons.more_vert, color: p.appBarFg),
      tooltip: "Chat options",
      onSelected: (value) {
        final st = _attachedHubChat;
        switch (value) {
          case "call":
            if (callPk != null) {
              unawaited(
                CallController.instance.startOutgoing(
                  context: context,
                  peerPublicKeyHex: callPk,
                  displayName: _selectedContactCallDisplayName(),
                ),
              );
            }
            break;
          case "share":
            if (!GhalBolFfi.isP2pAvailable) return;
            unawaited(_showMyQrInvitation());
            break;
          case "scan":
            if (!GhalBolFfi.isP2pAvailable) return;
            if (st != null) {
              unawaited(st.requestJoinFlow());
            } else {
              unawaited(_openJoinScan());
            }
            break;
          case "lock":
            widget.onUiLock();
            break;
        }
      },
      itemBuilder: (ctx) => [
        if (showCall)
          const PopupMenuItem(
            value: "call",
            child: ListTile(
              leading: Icon(Icons.call),
              title: Text("Voice call"),
              contentPadding: EdgeInsets.zero,
              visualDensity: VisualDensity.compact,
            ),
          ),
        const PopupMenuItem(
          value: "share",
          child: ListTile(
            leading: Icon(Icons.qr_code),
            title: Text("Share invitation"),
            contentPadding: EdgeInsets.zero,
            visualDensity: VisualDensity.compact,
          ),
        ),
        const PopupMenuItem(
          value: "scan",
          child: ListTile(
            leading: Icon(Icons.qr_code_scanner),
            title: Text("Join someone"),
            contentPadding: EdgeInsets.zero,
            visualDensity: VisualDensity.compact,
          ),
        ),
        const PopupMenuItem(
          value: "lock",
          child: ListTile(
            leading: Icon(Icons.lock_outline),
            title: Text("Lock"),
            contentPadding: EdgeInsets.zero,
            visualDensity: VisualDensity.compact,
          ),
        ),
      ],
    );
  }

  PreferredSizeWidget _listAppBar(BuildContext context, {required bool showLockInAppBar}) {
    final p = GhalBolChatRoomPalette.of(context);
    return AppBar(
      title: Text("Ghal Bol", style: TextStyle(color: p.appBarFg, fontWeight: FontWeight.w600)),
      backgroundColor: p.appBarBg,
      foregroundColor: p.appBarFg,
      surfaceTintColor: Colors.transparent,
      bottom: PreferredSize(
        preferredSize: const Size.fromHeight(1),
        child: Container(height: 1, color: p.appBarDivider.withValues(alpha: 0.45)),
      ),
      actions: [
        IconButton(
          tooltip: "New chat",
          onPressed: GhalBolFfi.isP2pAvailable ? _openNewChatMenu : null,
          icon: Icon(Icons.person_add_alt_1_outlined, color: p.appBarFg),
        ),
        IconButton(
          tooltip: "Show my QR invitation",
          onPressed: GhalBolFfi.isP2pAvailable ? () => unawaited(_showMyQrInvitation()) : null,
          icon: Icon(Icons.qr_code, color: p.appBarFg),
        ),
        IconButton(
          tooltip: "Scan to join someone",
          onPressed: () => unawaited(_openJoinScan()),
          icon: Icon(Icons.qr_code_scanner, color: p.appBarFg),
        ),
        if (showLockInAppBar)
          TextButton(
            onPressed: widget.onUiLock,
            child: Text("Lock", style: TextStyle(color: p.appBarFg)),
          ),
      ],
    );
  }

  Widget _chatListColumn(BuildContext context) {
    final colorScheme = Theme.of(context).colorScheme;
    final p = GhalBolChatRoomPalette.of(context);
    final filtered = _filteredContacts;

    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Padding(
          padding: const EdgeInsets.fromLTRB(12, 8, 12, 4),
          child: Material(
            color: p.isDark ? p.composerFieldFill : const Color(0xFFF0F2F5),
            borderRadius: BorderRadius.circular(24),
            child: Padding(
              padding: const EdgeInsets.symmetric(horizontal: 6),
              child: TextField(
                decoration: InputDecoration(
                  hintText: "Search contacts…",
                  border: InputBorder.none,
                  icon: Icon(Icons.search, size: 22, color: colorScheme.onSurfaceVariant),
                  isDense: true,
                ),
                onChanged: (v) => setState(() => _searchQuery = v),
              ),
            ),
          ),
        ),
        Expanded(
          child: filtered.isEmpty
              ? Center(
                  child: Padding(
                    padding: const EdgeInsets.all(20),
                    child: Column(
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        Icon(Icons.chat_outlined, size: 48, color: colorScheme.outline),
                        const SizedBox(height: 12),
                        Text(
                          _contacts.isEmpty ? "No chats yet" : "No matches",
                          style: Theme.of(context).textTheme.titleMedium,
                        ),
                        const SizedBox(height: 8),
                        Text(
                          _contacts.isEmpty
                              ? "Scan a QR or paste an invitation to start a 1:1 chat."
                              : "Try a different search.",
                          textAlign: TextAlign.center,
                          style: TextStyle(color: colorScheme.onSurfaceVariant),
                        ),
                      ],
                    ),
                  ),
                )
              : ListView.builder(
                  padding: const EdgeInsets.symmetric(vertical: 4),
                  itemCount: filtered.length,
                  itemBuilder: (ctx, i) {
                    final c = filtered[i];
                    final selected = c.conversationKey == _selectedConversationKey;
                    final hasUnread = c.unreadCount > 0;
                    final label = ghalBolIdName(
                      publicKeyHex: c.publicKeyHex,
                      customAlias: c.displayAlias,
                    );
                    final preview = c.lastMessagePreview ?? "Tap to open chat";
                    return DecoratedBox(
                      decoration: BoxDecoration(
                        color: selected && !ghalBolUseChatShellSplit(context) && _narrowShowRoom
                            ? colorScheme.primaryContainer.withValues(alpha: 0.35)
                            : selected && ghalBolUseChatShellSplit(context)
                                ? colorScheme.primaryContainer.withValues(alpha: 0.25)
                                : hasUnread
                                    ? colorScheme.secondaryContainer.withValues(alpha: 0.45)
                                    : Colors.transparent,
                        borderRadius: BorderRadius.circular(12),
                      ),
                      child: Material(
                        color: Colors.transparent,
                        child: InkWell(
                          onTap: () => unawaited(_selectContact(c)),
                          borderRadius: BorderRadius.circular(12),
                          child: Padding(
                            padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 10),
                            child: Row(
                              children: [
                                CircleAvatar(
                                  backgroundColor: colorScheme.primaryContainer,
                                  foregroundColor: colorScheme.onPrimaryContainer,
                                  child: Text(
                                    label.isNotEmpty ? label.substring(0, 1).toUpperCase() : "?",
                                  ),
                                ),
                                const SizedBox(width: 12),
                                Expanded(
                                  child: Column(
                                    crossAxisAlignment: CrossAxisAlignment.start,
                                    children: [
                                      Text(
                                        label,
                                        maxLines: 1,
                                        overflow: TextOverflow.ellipsis,
                                        style: Theme.of(context).textTheme.titleMedium?.copyWith(
                                              fontWeight: hasUnread ? FontWeight.w600 : FontWeight.w500,
                                            ),
                                      ),
                                      const SizedBox(height: 4),
                                      Text(
                                        preview,
                                        maxLines: 1,
                                        overflow: TextOverflow.ellipsis,
                                        style: Theme.of(context).textTheme.bodySmall?.copyWith(
                                              color: hasUnread
                                                  ? colorScheme.onSurface
                                                  : colorScheme.onSurfaceVariant,
                                              fontWeight: hasUnread ? FontWeight.w500 : FontWeight.normal,
                                            ),
                                      ),
                                    ],
                                  ),
                                ),
                                if (c.showUnknownChip)
                                  Padding(
                                    padding: const EdgeInsets.only(left: 6),
                                    child: FilledButton.tonal(
                                      onPressed: () => unawaited(_selectContact(c)),
                                      style: FilledButton.styleFrom(
                                        backgroundColor: colorScheme.tertiaryContainer,
                                        foregroundColor: colorScheme.onTertiaryContainer,
                                        padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
                                        minimumSize: Size.zero,
                                        tapTargetSize: MaterialTapTargetSize.shrinkWrap,
                                      ),
                                      child: const Text("Unknown"),
                                    ),
                                  ),
                                if (c.unreadCount > 0)
                                  Padding(
                                    padding: EdgeInsets.only(left: c.showUnknownChip ? 6 : 0),
                                    child: Container(
                                      padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 2),
                                      decoration: BoxDecoration(
                                        color: colorScheme.primary,
                                        borderRadius: BorderRadius.circular(12),
                                      ),
                                      child: Text(
                                        "${c.unreadCount}",
                                        style: TextStyle(color: colorScheme.onPrimary, fontSize: 12),
                                      ),
                                    ),
                                  ),
                              ],
                            ),
                          ),
                        ),
                      ),
                    );
                  },
                ),
        ),
      ],
    );
  }

  /// Desktop / wide: list pane with its own scaffold (inside split row).
  Widget _listPaneWide(BuildContext context, {required bool showLockInAppBar}) {
    final p = GhalBolChatRoomPalette.of(context);
    return ColoredBox(
      color: p.hubListBackground,
      child: Scaffold(
        backgroundColor: Colors.transparent,
        appBar: _listAppBar(context, showLockInAppBar: showLockInAppBar),
        body: _chatListColumn(context),
      ),
    );
  }

  Widget _identityBody(BuildContext context) {
    final peer = _s.publicKeyHex ?? "—";
    final pk = _s.publicKeyHex ?? "—";
    final ns = _s.appNamespace ?? "—";
    final colorScheme = Theme.of(context).colorScheme;

    return Material(
      color: colorScheme.surface,
      child: SafeArea(
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Padding(
              padding: const EdgeInsets.fromLTRB(16, 8, 16, 0),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  FilledButton.icon(
                    onPressed: GhalBolFfi.isP2pAvailable ? _openChatInvitationFromHub : null,
                    icon: const Icon(Icons.qr_code),
                    label: const Text("Show QR invitation"),
                  ),
                  const SizedBox(height: 8),
                  Wrap(
                    spacing: 8,
                    runSpacing: 8,
                    children: [
                      FilledButton.tonalIcon(
                        onPressed: GhalBolFfi.isP2pAvailable
                            ? () => unawaited(_shareInvitationFromHub(context))
                            : null,
                        icon: const Icon(Icons.share_outlined),
                        label: const Text("Share invitation"),
                      ),
                      FilledButton.tonalIcon(
                        onPressed: () => unawaited(_copyInvitationFromHub(context)),
                        icon: const Icon(Icons.link),
                        label: const Text("Copy invitation"),
                      ),
                      TextButton.icon(
                        onPressed: () => unawaited(_copyPublicKey(context)),
                        icon: const Icon(Icons.key_outlined, size: 18),
                        label: const Text("Copy public key"),
                      ),
                    ],
                  ),
                ],
              ),
            ),
            Expanded(
              child: SingleChildScrollView(
                padding: const EdgeInsets.fromLTRB(20, 12, 20, 28),
                child: SelectionArea(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text("Your identity", style: Theme.of(context).textTheme.titleLarge),
                      const SizedBox(height: 8),
                      Text(
                        "QR and invitation links encode only your secp256k1 public key (66 hex) — not IP addresses. "
                        "After they add you, the app finds you via the public DHT, same Wi‑Fi (mDNS), or when you connect to them.",
                        style: Theme.of(context).textTheme.bodySmall?.copyWith(
                              color: colorScheme.onSurfaceVariant,
                            ),
                      ),
                      const SizedBox(height: 12),
                      SelectableText("Namespace: $ns", style: Theme.of(context).textTheme.bodySmall),
                      const SizedBox(height: 12),
                      SelectableText("Public key (share this):\n$pk", style: Theme.of(context).textTheme.bodyMedium),
                      const SizedBox(height: 12),
                      SelectableText("libp2p PeerId (derived):\n$peer", style: Theme.of(context).textTheme.bodySmall),
                      if (GhalBolFfi.isIdentityKeyManagementAvailable) ...[
                        const Divider(height: 28),
                        Text("Backup & private key", style: Theme.of(context).textTheme.titleSmall),
                        const SizedBox(height: 8),
                        Wrap(
                          spacing: 8,
                          runSpacing: 8,
                          children: [
                            OutlinedButton.icon(
                              onPressed: () => unawaited(showRevealPrivateKeyDialog(context)),
                              icon: const Icon(Icons.vpn_key_outlined, size: 18),
                              label: const Text("Show private key"),
                            ),
                            OutlinedButton.icon(
                              onPressed: () => unawaited(exportKeystoreBackup(context)),
                              icon: const Icon(Icons.save_alt_outlined, size: 18),
                              label: const Text("Export backup"),
                            ),
                          ],
                        ),
                        const SizedBox(height: 4),
                        Text(
                          "App password is always required to view the secret. "
                          "You may import any supported key or backup; cryptocurrency wallet keys are not recommended. "
                          "You are responsible for what you import.",
                          style: Theme.of(context).textTheme.bodySmall?.copyWith(
                                color: colorScheme.onSurfaceVariant,
                              ),
                        ),
                      ],
                      if ((_s.publicKeyHex?.trim().length ?? 0) == 66 &&
                          isValidPublicKeyHex(_s.publicKeyHex)) ...[
                        const Divider(height: 28),
                        IdentityAliasForm(
                          appNamespace: _s.appNamespace ?? kGhalBolAppNamespace,
                          publicKeyHex: _s.publicKeyHex!.trim(),
                          onSaved: (v) {
                            setState(() {
                              _storedCustomAlias = v;
                              _aliasSaveNonce++;
                            });
                          },
                        ),
                      ],
                    ],
                  ),
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }

  Widget _moreBody(BuildContext context) {
    final colorScheme = Theme.of(context).colorScheme;
    return Material(
      color: colorScheme.surface,
      child: ListView(
        padding: const EdgeInsets.fromLTRB(8, 8, 8, 24),
        children: [
          ListTile(
            leading: Icon(Icons.info_outline, color: colorScheme.primary),
            title: const Text("About Ghal Bol"),
            subtitle: const SelectableText(
              "1:1 P2P encrypted chat with voice calls. No phone number or cloud account required.",
              maxLines: 4,
            ),
          ),
          const Divider(height: 1),
          ListTile(
            leading: Icon(Icons.contacts_outlined, color: colorScheme.primary),
            title: const Text("Contacts"),
            subtitle: const Text("Add, edit display names, or remove by public key"),
            onTap: () {
              Navigator.of(context).push(
                MaterialPageRoute<void>(
                  builder: (_) => ContactsScreen(appNamespace: _appNs),
                ),
              );
            },
          ),
          const Divider(height: 1),
          ListTile(
            leading: Icon(Icons.person_off_outlined, color: colorScheme.primary),
            title: const Text("Blocked contacts"),
            subtitle: const Text("People you blocked on this device"),
            onTap: () {
              Navigator.of(context).push(
                MaterialPageRoute<void>(
                  builder: (_) => BlockedPeersScreen(
                    appNamespace: _s.appNamespace ?? kGhalBolAppNamespace,
                  ),
                ),
              );
            },
          ),
          const Divider(height: 1),
          ListTile(
            leading: Icon(Icons.article_outlined, color: colorScheme.primary),
            title: const Text("App log"),
            subtitle: const Text("Session diagnostics log"),
            onTap: () {
              Navigator.of(context).push(
                MaterialPageRoute<void>(
                  builder: (_) => const AppLogScreen(),
                ),
              );
            },
          ),
          const Divider(height: 1),
          ListTile(
            leading: const Icon(Icons.lock_outline),
            title: const Text("Lock"),
            subtitle: const Text("Hide chats until you unlock (network stays on)"),
            onTap: widget.onUiLock,
          ),
          if (GhalBolFfi.isDeleteKeystoreAvailable) ...[
            const Divider(height: 1),
            ListTile(
              leading: Icon(Icons.delete_forever_outlined, color: colorScheme.error),
              title: Text("Delete identity from this device", style: TextStyle(color: colorScheme.error)),
              subtitle: const Text(
                "Stops chat, removes encrypted keys and saved display names. Requires your unlock password.",
              ),
              onTap: () => _confirmDeleteIdentityFromDevice(context),
            ),
          ],
        ],
      ),
    );
  }

  Future<void> _confirmDeleteIdentityFromDevice(BuildContext context) async {
    if (!GhalBolFfi.isDeleteKeystoreAvailable) {
      ScaffoldMessenger.of(context).showSnackBar(
        const SnackBar(content: Text("This build’s native library does not support delete. Re-sync libghal_bol.")),
      );
      return;
    }
    final passCtrl = TextEditingController();
    final go = await showDialog<bool>(
      context: context,
      barrierDismissible: false,
      builder: (ctx) => AlertDialog(
        title: const Text("Delete identity?"),
        content: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            const Text(
              "Stops chat and removes encrypted keys plus display-name preferences from this device. "
              "Enter your unlock password.",
            ),
            const SizedBox(height: 12),
            TextField(
              controller: passCtrl,
              obscureText: true,
              decoration: const InputDecoration(
                labelText: "Password",
                border: OutlineInputBorder(),
              ),
            ),
          ],
        ),
        actions: [
          TextButton(onPressed: () => Navigator.pop(ctx, false), child: const Text("Cancel")),
          FilledButton(
            style: FilledButton.styleFrom(backgroundColor: Theme.of(ctx).colorScheme.error),
            onPressed: () => Navigator.pop(ctx, true),
            child: const Text("Delete"),
          ),
        ],
      ),
    );
    final pw = passCtrl.text.trim();
    passCtrl.dispose();
    if (!context.mounted || go != true || pw.isEmpty) return;
    await GhalBolBackground.stopForLogout();
    final ns = _s.appNamespace ?? kGhalBolAppNamespace;
    final r = GhalBolFfi.deleteKeystoreVerified(appNamespace: ns, password: pw);
    if (!context.mounted) return;
    if (!r.ok) {
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text(r.error ?? "Delete failed")),
      );
      return;
    }
    widget.onEndSession();
  }

  Widget _chatBody() {
    // Prefer 66-hex public key; legacy libp2p PeerId strings must not be validated as hex.
    final localPk = resolvePublicKeyHex(storedHex: _s.publicKeyHex);
    final localId = isValidPublicKeyHex(localPk) ? localPk! : "";
    if (localId.isEmpty) {
      return const Center(
        child: Padding(
          padding: EdgeInsets.all(24),
          child: Text(
            "Missing identity — lock and unlock again.",
            textAlign: TextAlign.center,
          ),
        ),
      );
    }
    final contact = _selectedContact;
    final convKey = _selectedConversationKey ?? contact?.conversationKey ?? "none";
    final split = ghalBolUseChatShellSplit(context);
    return ChatScreen(
      key: ValueKey("hub-chat-$convKey"),
      libp2pPeerId: localId,
      publicKeyHex: isValidPublicKeyHex(localPk) ? localPk : _s.publicKeyHex,
      appNamespace: _appNs,
      localPeerAlias: _storedCustomAlias,
      aliasNonce: _aliasSaveNonce,
      hubThreadKey: _selectedConversationKey,
      activeContact: contact,
      hubPollsEvents: true,
      onHubChatAttach: _attachHubChat,
      onHubChatDetach: _detachHubChat,
      hubPeerStreamReady: _hubPeerStreamReady,
      onContactJoined: _onContactJoined,
      onLeaveRoom: split ? null : _popHubChromeBack,
      onLock: null,
      networkActionsInHub: true,
    );
  }

  static const _navDestinations = <NavigationRailDestination>[
    NavigationRailDestination(
      icon: Icon(Icons.chat_bubble_outline),
      selectedIcon: Icon(Icons.chat_bubble),
      label: Text("Chats"),
    ),
    NavigationRailDestination(
      icon: Icon(Icons.badge_outlined),
      selectedIcon: Icon(Icons.badge),
      label: Text("Identity"),
    ),
    NavigationRailDestination(
      icon: Icon(Icons.more_horiz),
      selectedIcon: Icon(Icons.more_horiz),
      label: Text("More"),
    ),
  ];

  Widget _wideShell(BuildContext context) {
    final w = MediaQuery.sizeOf(context).width;
    final listWidth = (w * 0.34).clamp(280.0, 400.0);
    final colorScheme = Theme.of(context).colorScheme;
    final p = GhalBolChatRoomPalette.of(context);
    final railExtended = MediaQuery.sizeOf(context).width >= 1100;

    return Scaffold(
      body: Row(
        children: [
          NavigationRail(
          extended: railExtended,
          backgroundColor: p.appBarBg,
          selectedIndex: _navTab,
          selectedIconTheme: IconThemeData(color: colorScheme.primary, size: 26),
          selectedLabelTextStyle: TextStyle(
            color: colorScheme.primary,
            fontSize: 11,
            fontWeight: FontWeight.w600,
          ),
          unselectedIconTheme: IconThemeData(color: p.metaText, size: 24),
          unselectedLabelTextStyle: TextStyle(color: p.metaText, fontSize: 11),
          labelType: railExtended ? NavigationRailLabelType.none : NavigationRailLabelType.all,
          onDestinationSelected: _onHubNavTabSelected,
          leading: Padding(
            padding: const EdgeInsets.only(top: 8, bottom: 12),
            child: ClipRRect(
              borderRadius: BorderRadius.circular(8),
              child: Image.asset(
                "assets/app_icon.png",
                width: railExtended ? 28 : 26,
                height: railExtended ? 28 : 26,
                fit: BoxFit.cover,
                errorBuilder: (context, error, stackTrace) => Icon(
                  Icons.chat_bubble,
                  color: colorScheme.primary,
                  size: railExtended ? 28 : 26,
                ),
              ),
            ),
          ),
          trailing: Padding(
            padding: const EdgeInsets.only(bottom: 8),
            child: IconButton(
              tooltip: "Lock",
              onPressed: widget.onUiLock,
              icon: Icon(Icons.lock_outline, color: p.appBarFg),
            ),
          ),
          destinations: _navDestinations,
          ),
          VerticalDivider(width: 1, thickness: 1, color: colorScheme.outlineVariant),
          Expanded(
            child: switch (_navTab) {
              0 => Row(
                children: [
                  SizedBox(width: listWidth, child: _listPaneWide(context, showLockInAppBar: false)),
                  VerticalDivider(width: 1, thickness: 1, color: colorScheme.outlineVariant),
                  Expanded(
                    child: Listener(
                      behavior: HitTestBehavior.translucent,
                      onPointerDown: (_) {
                        if (!_splitChatEngaged) {
                          setState(() => _splitChatEngaged = true);
                          _recordHubNavigation();
                          _syncNativeForegroundPeer();
                        }
                      },
                      child: _chatBody(),
                    ),
                  ),
                ],
              ),
              1 => _identityBody(context),
              _ => _moreBody(context),
            },
          ),
        ],
      ),
    );
  }

  PreferredSizeWidget? _narrowAppBar(BuildContext context) {
    switch (_navTab) {
      case 0:
        if (_narrowShowRoom) {
          final p = GhalBolChatRoomPalette.of(context);
          return AppBar(
            leading: BackButton(
              color: p.appBarFg,
              onPressed: _popHubChromeBack,
            ),
            title: Text(
              _selectedContact != null
                  ? ghalBolIdName(
                      publicKeyHex: _selectedContact!.publicKeyHex,
                      customAlias: _selectedContact!.displayAlias,
                    )
                  : "Chats",
              style: TextStyle(color: p.appBarFg, fontWeight: FontWeight.w600),
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
            ),
            backgroundColor: p.appBarBg,
            foregroundColor: p.appBarFg,
            surfaceTintColor: Colors.transparent,
            bottom: PreferredSize(
              preferredSize: const Size.fromHeight(1),
              child: Container(height: 1, color: p.appBarDivider.withValues(alpha: 0.45)),
            ),
            actions: [_hubChatRoomPopupMenu(context, p)],
          );
        }
        return _listAppBar(context, showLockInAppBar: true);
      case 1:
        final p = GhalBolChatRoomPalette.of(context);
        return AppBar(
          title: Text("Identity", style: TextStyle(color: p.appBarFg, fontWeight: FontWeight.w600)),
          backgroundColor: p.appBarBg,
          foregroundColor: p.appBarFg,
          surfaceTintColor: Colors.transparent,
          bottom: PreferredSize(
            preferredSize: const Size.fromHeight(1),
            child: Container(height: 1, color: p.appBarDivider.withValues(alpha: 0.45)),
          ),
          actions: [
            TextButton(
              onPressed: widget.onUiLock,
              child: Text("Lock", style: TextStyle(color: p.appBarFg)),
            ),
          ],
        );
      case 2:
        final p2 = GhalBolChatRoomPalette.of(context);
        return AppBar(
          title: Text("More", style: TextStyle(color: p2.appBarFg, fontWeight: FontWeight.w600)),
          backgroundColor: p2.appBarBg,
          foregroundColor: p2.appBarFg,
          surfaceTintColor: Colors.transparent,
          bottom: PreferredSize(
            preferredSize: const Size.fromHeight(1),
            child: Container(height: 1, color: p2.appBarDivider.withValues(alpha: 0.45)),
          ),
        );
      default:
        return null;
    }
  }

  Widget _narrowChatsLayer() {
    final p = GhalBolChatRoomPalette.of(context);
    return IndexedStack(
      index: _narrowShowRoom ? 1 : 0,
      sizing: StackFit.expand,
      children: [
        ColoredBox(
          color: p.hubListBackground,
          child: _chatListColumn(context),
        ),
        _chatBody(),
      ],
    );
  }

  Widget _narrowShell(BuildContext context) {
    final showBottomNav = !(_navTab == 0 && _narrowShowRoom);
    final p = GhalBolChatRoomPalette.of(context);

    return Scaffold(
      backgroundColor: _navTab == 0 && _narrowShowRoom ? p.chatBackground : p.hubListBackground,
      appBar: _narrowAppBar(context),
      floatingActionButton: _navTab == 0 && !_narrowShowRoom && _selectedContact == null && GhalBolFfi.isP2pAvailable
          ? FloatingActionButton(
              onPressed: _openNewChatMenu,
              tooltip: "New chat",
              child: const Icon(Icons.add),
            )
          : null,
      floatingActionButtonLocation: FloatingActionButtonLocation.endFloat,
      body: IndexedStack(
        index: _navTab,
        sizing: StackFit.expand,
        children: [
          _narrowChatsLayer(),
          _identityBody(context),
          _moreBody(context),
        ],
      ),
      bottomNavigationBar: showBottomNav
          ? NavigationBar(
              selectedIndex: _navTab,
              onDestinationSelected: _onHubNavTabSelected,
              height: 64,
              backgroundColor: p.composerBar,
              elevation: 8,
              shadowColor: Colors.black26,
              surfaceTintColor: Colors.transparent,
              labelBehavior: NavigationDestinationLabelBehavior.alwaysShow,
              indicatorColor: p.isDark
                  ? Colors.white.withValues(alpha: 0.12)
                  : Colors.white.withValues(alpha: 0.95),
              destinations: const [
                NavigationDestination(
                  icon: Icon(Icons.chat_bubble_outline),
                  selectedIcon: Icon(Icons.chat_bubble),
                  label: "Chats",
                ),
                NavigationDestination(
                  icon: Icon(Icons.badge_outlined),
                  selectedIcon: Icon(Icons.badge),
                  label: "Identity",
                ),
                NavigationDestination(
                  icon: Icon(Icons.more_horiz),
                  label: "More",
                ),
              ],
            )
          : null,
    );
  }

  @override
  Widget build(BuildContext context) {
    _syncNativeForegroundIfLayoutChanged(context);
    final split = _hubShellSplit(context);
    return split ? _wideShell(context) : _narrowShell(context);
  }
}
