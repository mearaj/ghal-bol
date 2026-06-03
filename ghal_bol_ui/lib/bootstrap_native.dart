import "dart:async";

import "package:flutter/foundation.dart";
import "package:flutter/material.dart";
import "package:flutter/services.dart";

import "package:ghal_bol_ui/ghal_bol_ffi.dart";

import "call/call_controller.dart";
import "chat_hub_screen.dart";
import "chat_screen.dart";
import "ghal_bol_background.dart";
import "ghal_bol_daemon.dart";
import "ghal_bol_constants.dart";
import "ghal_bol_host_init.dart";
import "identity_alias_form.dart";
import "identity_alias_store.dart";
import "identity_key_management.dart";
import "identity_setup_copy.dart";
import "invite_uri_builder.dart";
import "public_key_hex.dart";
import "secret_key_hex.dart";
import "session_credentials.dart";
import "p2p_event_bridge.dart";
import "p2p_network_coordinator.dart";
import "app_log.dart";
import "invite_deep_link.dart";
import "user_flow_log.dart";

Future<void> runGhalBol() async {
  WidgetsFlutterBinding.ensureInitialized();
  await InviteDeepLink.install();
  if (kDebugMode) {
    // Forward Rust `native_log::debug` (libp2p tick detail) into the App log.
    AppLog.logNativeDebug = true;
    ErrorWidget.builder = (FlutterErrorDetails details) {
      final text =
          "${details.exceptionAsString()}\n\n${details.stack?.toString().trim() ?? "(no stack)"}";
      return Material(
        color: const Color(0xFFB71C1C),
        child: SafeArea(
          child: Scrollbar(
            thumbVisibility: true,
            child: SingleChildScrollView(
              padding: const EdgeInsets.all(14),
              child: SelectionArea(
                child: SelectableText(
                  text,
                  style: const TextStyle(
                    color: Color(0xFFFFF59D),
                    fontSize: 13,
                    height: 1.45,
                    fontFamily: "monospace",
                  ),
                ),
              ),
            ),
          ),
        ),
      );
    };
  }
  await ghalBolHostInitBeforeRunApp();
  runApp(const GhalBolApp());
}

class GhalBolApp extends StatelessWidget {
  const GhalBolApp({super.key});

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      navigatorKey: CallController.navigatorKey,
      title: "Ghal Bol",
      theme: ThemeData(
        colorScheme: ColorScheme.fromSeed(seedColor: Colors.deepPurple),
        useMaterial3: true,
      ),
      home: const GhalBolRoot(),
    );
  }
}

/// Chooses identity vs post-unlock chat shell (split or stacked from **window width** only).
class GhalBolRoot extends StatefulWidget {
  const GhalBolRoot({super.key});

  @override
  State<GhalBolRoot> createState() => _GhalBolRootState();
}

class _GhalBolRootState extends State<GhalBolRoot> {
  GhalBolIdentityResult? _session;
  /// Hides chat UI only — P2P poll, daemon, and hub state stay alive.
  bool _uiLocked = false;
  final _chatHubKey = GlobalKey<ChatHubScreenState>();

  @override
  void dispose() {
    unawaited(InviteDeepLink.dispose());
    super.dispose();
  }

  void _onRootSystemBackInvoked(bool didPop) {
    if (didPop) return;
    if (_session == null) {
      SystemNavigator.pop();
      return;
    }
    if (_uiLocked) {
      setState(() => _uiLocked = false);
      return;
    }
    if (_chatHubKey.currentState?.handleHubSystemBack() ?? false) return;
    SystemNavigator.pop();
  }

  @override
  Widget build(BuildContext context) {
    if (_session == null) {
      return PopScope(
        canPop: false,
        onPopInvokedWithResult: (didPop, _) => _onRootSystemBackInvoked(didPop),
        child: IdentityScreen(
          onUnlockedSession: (GhalBolIdentityResult r) {
            if (!mounted) return;
            unawaited(GhalBolBackground.ensureRunning(r));
            setState(() {
              _session = r;
              _uiLocked = false;
            });
          },
        ),
      );
    }
    return PopScope(
      canPop: false,
      onPopInvokedWithResult: (didPop, _) => _onRootSystemBackInvoked(didPop),
      child: Stack(
        fit: StackFit.expand,
        children: [
          Offstage(
            offstage: _uiLocked,
            child: ChatHubScreen(
              key: _chatHubKey,
              session: _session!,
              onUiLock: () {
                if (!mounted) return;
                setState(() => _uiLocked = true);
              },
              onEndSession: () {
                if (!mounted) return;
                setState(() {
                  _session = null;
                  _uiLocked = false;
                });
              },
            ),
          ),
          if (_uiLocked)
            IdentityScreen(
              uiLockResume: true,
              onUiLockResume: () {
                if (!mounted) return;
                setState(() => _uiLocked = false);
              },
            ),
        ],
      ),
    );
  }
}

class IdentityScreen extends StatefulWidget {
  const IdentityScreen({
    super.key,
    this.onUnlockedSession,
    this.uiLockResume = false,
    this.onUiLockResume,
  });

  /// Full sign-in / first-time setup → chat shell.
  final ValueChanged<GhalBolIdentityResult>? onUnlockedSession;

  /// Password gate to show chats again after UI lock (does not stop P2P).
  final bool uiLockResume;
  final VoidCallback? onUiLockResume;

  @override
  State<IdentityScreen> createState() => _IdentityScreenState();
}

class _IdentityScreenState extends State<IdentityScreen> {
  final _passwordCtrl = TextEditingController();
  final _secretKeyCtrl = TextEditingController();
  GhalBolIdentityResult? _last;
  bool _busy = false;
  bool? _keystoreOnDisk;
  bool _obscurePassword = true;
  /// When no keystore: `true` = generate new key; `false` = import 64-hex secret.
  bool _createNewIdentity = true;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) => _refreshKeystoreOnDisk());
  }

  void _refreshKeystoreOnDisk() {
    if (!mounted) return;
    setState(() {
      _keystoreOnDisk = GhalBolFfi.keystoreExists(appNamespace: kGhalBolAndroidLibraryNamespace);
    });
  }

  String _primaryAuthButtonLabel(bool nativeLoaded) {
    if (!nativeLoaded) return "Continue";
    if (_keystoreOnDisk == true) return "Unlock";
    if (_keystoreOnDisk == false) return _createNewIdentity ? "Create identity" : "Import identity";
    return "Continue";
  }

  @override
  void dispose() {
    _passwordCtrl.dispose();
    _secretKeyCtrl.dispose();
    super.dispose();
  }

  void _copyToClipboard(BuildContext context, String label, String text) {
    Clipboard.setData(ClipboardData(text: text));
    ScaffoldMessenger.of(context).showSnackBar(
      SnackBar(content: Text("$label copied")),
    );
  }

  Future<GhalBolIdentityResult> _unlockOrImportIdentity({
    required String ns,
    required String password,
  }) async {
    if (_keystoreOnDisk == false &&
        !_createNewIdentity &&
        GhalBolFfi.isIdentityKeyManagementAvailable) {
      final sk = normalizeSecretKeyHex(_secretKeyCtrl.text);
      if (!isValidSecretKeyHex(sk)) {
        return const GhalBolIdentityResult(
          ok: false,
          error: "Private key must be 64 hex characters (32-byte secp256k1 secret).",
        );
      }
      return GhalBolFfi.importIdentityFromSecretHex(
        appNamespace: ns,
        password: password,
        secretKeyHex: sk,
      );
    }
    return GhalBolFfi.createOrUnlockIdentity(appNamespace: ns, password: password);
  }

  static const String _firstTimeRetryHint = IdentitySetupCopy.firstTimeRetryHint;

  Future<void> _recoverFirstTimeSetupFailed() async {
    GhalBolFfi.resetFirstTimeIdentity(appNamespace: kGhalBolAndroidLibraryNamespace);
    GhalBolFfi.lock();
    if (GhalBolDaemon.isSupported) {
      await GhalBolDaemon.stopSession();
    }
    _passwordCtrl.clear();
    if (mounted) {
      setState(() => _keystoreOnDisk = false);
    }
  }

  GhalBolIdentityResult _withFirstTimeRetryHint(GhalBolIdentityResult r) {
    if (r.ok) return r;
    final err = r.error?.trim() ?? "Setup failed";
    if (err.endsWith(_firstTimeRetryHint.trim())) return r;
    return GhalBolIdentityResult(ok: false, error: "$err$_firstTimeRetryHint");
  }

  /// Unblocks the chat shell immediately after FFI unlock; P2P process unlock runs in parallel.
  Future<void> _finishDaemonUnlockAfterLogin({
    required String ns,
    required String password,
    required String? ffiPublicKeyHex,
    required bool firstTimeSetup,
  }) async {
    final dr = await GhalBolDaemon.unlock(
      appNamespace: ns,
      password: password,
    );
    if (dr["ok"] != true) {
      SessionFlowLog.daemonIssue(
        "daemon_unlock_deferred",
        check: "P2pEventBridge recoverP2pIfNeeded",
        detail: dr["error"]?.toString(),
      );
      unawaited(P2pEventBridge.instance.recoverP2pIfNeeded());
      return;
    }
    if (!publicKeysEqual(ffiPublicKeyHex, dr["public_key_hex"]?.toString())) {
      SessionFlowLog.issue(
        "identity_split",
        check: "clear app data; rebuild native; FFI vs daemon data dir",
        detail: "ffi pk != daemon pk",
      );
      if (firstTimeSetup) {
        await _recoverFirstTimeSetupFailed();
      } else {
        await GhalBolDaemon.stopSession();
        GhalBolFfi.lock();
      }
      return;
    }
    SessionFlowLog.daemon("daemon_unlock_ok");
    P2pNetworkCoordinator.markSessionRefresh();
    unawaited(P2pEventBridge.instance.recoverP2pIfNeeded());
  }

  Future<bool> _confirmImportPrivateKeyWarning(BuildContext context) async {
    final go = await showDialog<bool>(
      context: context,
      builder: (ctx) => AlertDialog(
        title: const Text(IdentitySetupCopy.importPrivateKeyWarningTitle),
        content: const Text(IdentitySetupCopy.importPrivateKeyWarningBody),
        actions: [
          TextButton(onPressed: () => Navigator.pop(ctx, false), child: const Text("Go back")),
          FilledButton(onPressed: () => Navigator.pop(ctx, true), child: const Text("Continue")),
        ],
      ),
    );
    return go == true;
  }

  /// UI lock resume: verify password only; daemon/P2P keep running.
  Future<void> _resumeFromUiLock() async {
    final password = _passwordCtrl.text;
    if (password.isEmpty) {
      if (mounted) {
        setState(() {
          _last = const GhalBolIdentityResult(
            ok: false,
            error: "App password is required.",
          );
        });
      }
      return;
    }
    setState(() {
      _busy = true;
      _last = null;
    });
    const ns = kGhalBolAndroidLibraryNamespace;
    SessionFlowLog.step("ui_lock_resume");
    try {
      final ffi = GhalBolFfi.createOrUnlockIdentity(appNamespace: ns, password: password);
      if (!ffi.ok) {
        SessionFlowLog.issue("ffi_unlock_failed", detail: ffi.error ?? "unknown");
        if (mounted) setState(() => _last = ffi);
        return;
      }
      SessionFlowLog.step("ffi_unlock_ok", {"pk": SessionFlowLog.shortPk(ffi.publicKeyHex)});
      SessionCredentials.store(appNamespace: ns, password: password);
      if (GhalBolDaemon.isSupported && !await GhalBolDaemon.sessionUnlocked()) {
        await GhalBolDaemon.prepareForLoginUnlock();
        final dr = await GhalBolDaemon.unlockWithRecovery(
          appNamespace: ns,
          password: password,
        );
        if (dr["ok"] != true) {
          SessionFlowLog.daemonIssue(
            "daemon_unlock_deferred",
            check: "P2pEventBridge may re-unlock; grep Daemon step=",
            detail: dr["error"]?.toString(),
          );
        } else {
          SessionFlowLog.daemon("daemon_unlock_ok", {"mode": "ui_lock_resume"});
        }
      }
      widget.onUiLockResume?.call();
      _passwordCtrl.clear();
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  Future<void> _unlock() async {
    if (widget.uiLockResume) {
      await _resumeFromUiLock();
      return;
    }
    if (_keystoreOnDisk == false &&
        !_createNewIdentity &&
        GhalBolFfi.isIdentityKeyManagementAvailable) {
      final ok = await _confirmImportPrivateKeyWarning(context);
      if (!mounted || !ok) return;
    }
    setState(() {
      _busy = true;
      _last = null;
    });
    final password = _passwordCtrl.text;
    if (password.isEmpty) {
      if (mounted) {
        setState(() {
          _busy = false;
          _last = const GhalBolIdentityResult(
            ok: false,
            error: "App password is required.",
          );
        });
      }
      return;
    }
    const ns = kGhalBolAndroidLibraryNamespace;
    final firstTimeSetup = _keystoreOnDisk == false;
    SessionFlowLog.step("login_submit", {
      "first_time": firstTimeSetup.toString(),
      "import_key": (!_createNewIdentity && _keystoreOnDisk == false).toString(),
      "daemon": GhalBolDaemon.isSupported.toString(),
    });
    try {
      GhalBolIdentityResult r;
      if (GhalBolDaemon.isSupported && widget.onUnlockedSession != null) {
        SessionFlowLog.daemon("prepare_login");
        await GhalBolDaemon.prepareForLoginUnlock();
        SessionFlowLog.step("ffi_unlock_start");
        final ffi = await _unlockOrImportIdentity(ns: ns, password: password);
        if (!ffi.ok) {
          SessionFlowLog.issue("ffi_unlock_failed", detail: ffi.error ?? "unknown");
          if (firstTimeSetup) {
            await _recoverFirstTimeSetupFailed();
            if (mounted) setState(() => _last = _withFirstTimeRetryHint(ffi));
          } else if (mounted) {
            setState(() => _last = ffi);
          }
          return;
        }
        SessionFlowLog.step("ffi_unlock_ok", {"pk": SessionFlowLog.shortPk(ffi.publicKeyHex)});
        SessionCredentials.store(appNamespace: ns, password: password);
        r = GhalBolIdentityResult(
          ok: true,
          publicKeyHex: ffi.publicKeyHex,
          libp2pPeerId: ffi.libp2pPeerId,
          appNamespace: ns,
        );
        SessionFlowLog.daemon("daemon_unlock_background");
        unawaited(_finishDaemonUnlockAfterLogin(
          ns: ns,
          password: password,
          ffiPublicKeyHex: ffi.publicKeyHex,
          firstTimeSetup: firstTimeSetup,
        ));
      } else {
        SessionFlowLog.step("ffi_unlock_start", {"daemon": "false"});
        r = await _unlockOrImportIdentity(ns: ns, password: password);
        if (firstTimeSetup && !r.ok) {
          SessionFlowLog.issue("first_time_setup_failed", detail: r.error);
          await _recoverFirstTimeSetupFailed();
          r = _withFirstTimeRetryHint(r);
        } else if (r.ok) {
          SessionFlowLog.step("ffi_unlock_ok", {"pk": SessionFlowLog.shortPk(r.publicKeyHex)});
        } else {
          SessionFlowLog.issue("ffi_unlock_failed", detail: r.error);
        }
      }
      if (mounted) {
        if (r.ok) {
          if (!GhalBolDaemon.isSupported || widget.onUnlockedSession == null) {
            SessionCredentials.store(appNamespace: ns, password: password);
          }
          SessionFlowLog.step("session_unlocked", {
            "pk": SessionFlowLog.shortPk(r.publicKeyHex),
            "peer_id": r.libp2pPeerId ?? "?",
          });
        }
        if (r.ok && widget.onUnlockedSession != null) {
          SessionFlowLog.step("enter_app");
          widget.onUnlockedSession!(r);
        } else {
          setState(() {
            _last = r;
            if (r.ok) {
              _keystoreOnDisk = true;
            }
          });
        }
      }
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  Future<void> _logout() async {
    await GhalBolBackground.stopForLogout();
    GhalBolFfi.lock();
    _passwordCtrl.clear();
    setState(() => _last = null);
    _refreshKeystoreOnDisk();
  }

  Future<void> _offerDeleteIdentityFromUnlockScreen(BuildContext context) async {
    if (!GhalBolFfi.isDeleteKeystoreAvailable) return;
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
              "Removes encrypted keys and saved display names from this device. "
              "Enter the same password you use to unlock.",
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
    setState(() => _busy = true);
    try {
      await GhalBolBackground.stopForLogout();
      final r = GhalBolFfi.deleteKeystoreVerified(
        appNamespace: kGhalBolAndroidLibraryNamespace,
        password: pw,
      );
      if (!context.mounted) return;
      if (!r.ok) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(content: Text(r.error ?? "Delete failed")),
        );
        return;
      }
      _passwordCtrl.clear();
      setState(() => _last = null);
      _refreshKeystoreOnDisk();
      ScaffoldMessenger.of(context).showSnackBar(
        const SnackBar(content: Text("Identity removed from this device.")),
      );
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final loaded = GhalBolFfi.isLibraryLoaded;
    final loadErr = GhalBolFfi.loadErrorText ?? "";

    return Scaffold(
      resizeToAvoidBottomInset: true,
      appBar: AppBar(
        title: Text(widget.uiLockResume ? "Chats locked" : "Ghal Bol identity"),
        actions: [
          if (!widget.uiLockResume)
            TextButton(onPressed: _busy ? null : () => _logout(), child: const Text("Lock")),
        ],
      ),
      body: SafeArea(
        child: LayoutBuilder(
          builder: (context, c) {
            final scroll = SingleChildScrollView(
              padding: const EdgeInsets.fromLTRB(20, 12, 20, 24),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  if (widget.uiLockResume) ...[
                    Text(
                      "Messages, calls, and P2P keep running while the chat UI is hidden.",
                      style: Theme.of(context).textTheme.bodyMedium,
                    ),
                    const SizedBox(height: 16),
                  ],
                  if (!widget.uiLockResume) ...[
                    Row(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Expanded(
                          child: SelectableText(
                            loaded ? "Keystore native: ok." : "Keystore native: unavailable.\n$loadErr",
                            style: Theme.of(context).textTheme.bodySmall,
                            maxLines: 16,
                          ),
                        ),
                        if (!loaded && loadErr.isNotEmpty)
                          IconButton(
                            tooltip: "Copy",
                            onPressed: () => _copyToClipboard(context, "Error", loadErr),
                            icon: const Icon(Icons.copy, size: 20),
                          ),
                      ],
                    ),
                    const SizedBox(height: 16),
                  ],
                  if (!widget.uiLockResume && loaded && _keystoreOnDisk == false) ...[
                    IdentityFirstSetupBanner(importMode: !_createNewIdentity),
                    const SizedBox(height: 12),
                    SegmentedButton<bool>(
                      segments: const [
                        ButtonSegment(value: true, label: Text("Create new")),
                        ButtonSegment(value: false, label: Text("Import key")),
                      ],
                      selected: {_createNewIdentity},
                      onSelectionChanged: _busy
                          ? null
                          : (s) => setState(() => _createNewIdentity = s.first),
                    ),
                    const SizedBox(height: 12),
                  ],
                  if (!widget.uiLockResume && loaded && _keystoreOnDisk == false && !_createNewIdentity) ...[
                    TextField(
                      controller: _secretKeyCtrl,
                      maxLines: 2,
                      decoration: const InputDecoration(
                        border: OutlineInputBorder(),
                        labelText: "Private key (64 hex)",
                        hintText: "Any valid secp256k1 secret — wallet keys not recommended",
                      ),
                    ),
                    const SizedBox(height: 12),
                  ],
                  TextField(
                    controller: _passwordCtrl,
                    obscureText: _obscurePassword,
                    decoration: InputDecoration(
                      border: const OutlineInputBorder(),
                      labelText: _keystoreOnDisk == true
                          ? "App password (required)"
                          : "Choose app password (required)",
                      helperText: _keystoreOnDisk == false
                          ? "Required to encrypt and unlock your identity on this device."
                          : null,
                      helperMaxLines: 2,
                      suffixIcon: IconButton(
                        tooltip: _obscurePassword ? "Show password" : "Hide password",
                        onPressed: () => setState(() => _obscurePassword = !_obscurePassword),
                        icon: Icon(
                          _obscurePassword ? Icons.visibility_outlined : Icons.visibility_off_outlined,
                        ),
                      ),
                    ),
                    onSubmitted: (_) => _unlock(),
                  ),
                  const SizedBox(height: 12),
                  if (loaded && _keystoreOnDisk != null) ...[
                    Text(
                      _keystoreOnDisk!
                          ? "Saved identity found on this device."
                          : "No saved identity on this device yet.",
                      style: Theme.of(context).textTheme.bodySmall?.copyWith(
                        color: Theme.of(context).colorScheme.outline,
                      ),
                    ),
                    const SizedBox(height: 8),
                  ],
                  FilledButton(
                    onPressed: _busy ? null : _unlock,
                    child: Text(
                      _busy
                          ? "…"
                          : widget.uiLockResume
                              ? "Show chats"
                              : _primaryAuthButtonLabel(loaded),
                    ),
                  ),
                  if (!widget.uiLockResume &&
                      loaded &&
                      _keystoreOnDisk == false &&
                      GhalBolFfi.isIdentityKeyManagementAvailable) ...[
                    const SizedBox(height: 8),
                    TextButton.icon(
                      onPressed: _busy
                          ? null
                          : () => importKeystoreBackup(
                                context,
                                onImported: () {
                                  _refreshKeystoreOnDisk();
                                  _passwordCtrl.clear();
                                },
                              ),
                      icon: const Icon(Icons.upload_file_outlined),
                      label: const Text("Import encrypted keystore backup"),
                    ),
                  ],
                  if (loaded &&
                      GhalBolFfi.isDeleteKeystoreAvailable &&
                      _keystoreOnDisk == true) ...[
                    const SizedBox(height: 16),
                    Align(
                      alignment: Alignment.centerLeft,
                      child: TextButton.icon(
                        onPressed: _busy ? null : () => _offerDeleteIdentityFromUnlockScreen(context),
                        icon: Icon(Icons.delete_forever_outlined, color: Theme.of(context).colorScheme.error),
                        label: Text(
                          "Delete identity from this device",
                          style: TextStyle(color: Theme.of(context).colorScheme.error),
                        ),
                      ),
                    ),
                  ],
                  const SizedBox(height: 24),
                  if (_last != null) _buildResult(context, _last!),
                ],
              ),
            );
            if (c.maxWidth >= 600) {
              return Align(
                alignment: Alignment.topCenter,
                child: ConstrainedBox(
                  constraints: const BoxConstraints(maxWidth: 520),
                  child: scroll,
                ),
              );
            }
            return scroll;
          },
        ),
      ),
    );
  }

  Widget _buildResult(BuildContext context, GhalBolIdentityResult r) {
    if (!r.ok) {
      final msg = r.error ?? "Error";
      return Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Expanded(
            child: SelectableText(
              msg,
              style: TextStyle(color: Theme.of(context).colorScheme.error),
              maxLines: 24,
            ),
          ),
          IconButton(
            tooltip: "Copy",
            onPressed: () => _copyToClipboard(context, "Error", msg),
            icon: const Icon(Icons.copy, size: 20),
          ),
        ],
      );
    }
    final pk = r.publicKeyHex ?? "—";
    final peer = r.libp2pPeerId ?? "—";
    return SelectionArea(
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(
            "Unlocked (${r.appNamespace ?? kGhalBolAndroidLibraryNamespace})",
            style: Theme.of(context).textTheme.titleSmall,
          ),
          const SizedBox(height: 8),
          SelectableText("libp2p PeerId:\n$peer", maxLines: 8),
          const SizedBox(height: 8),
          SelectableText("secp256k1 public key:\n$pk", maxLines: 4),
          if ((r.publicKeyHex?.trim().length ?? 0) == 66 &&
              (r.libp2pPeerId != null && r.libp2pPeerId!.isNotEmpty)) ...[
            const SizedBox(height: 12),
            TextButton.icon(
              onPressed: () async {
                final pk = r.publicKeyHex?.trim() ?? "";
                if (!isValidPublicKeyHex(pk)) return;
                final ns = r.appNamespace ?? kGhalBolAndroidLibraryNamespace;
                final alias = await IdentityAliasStore.read(
                  appNamespace: ns,
                  publicKeyHex: pk,
                );
                final uri = buildGhalBolInviteUri(
                  publicKeyHex: pk,
                  peerAlias: alias,
                );
                if (uri == null) return;
                await Clipboard.setData(ClipboardData(text: uri));
                if (!context.mounted) return;
                ScaffoldMessenger.of(context).showSnackBar(
                  const SnackBar(content: Text("Invitation copied")),
                );
              },
              icon: const Icon(Icons.link, size: 18),
              label: const Text("Copy invitation"),
            ),
            const SizedBox(height: 20),
            IdentityAliasForm(
              appNamespace: r.appNamespace ?? kGhalBolAndroidLibraryNamespace,
              publicKeyHex: r.publicKeyHex!.trim(),
              onSaved: (_) {},
            ),
          ],
          const SizedBox(height: 20),
          if (widget.onUnlockedSession == null) ...[
            SizedBox(
              width: double.infinity,
              child: FilledButton.icon(
                onPressed: (isValidPublicKeyHex(r.publicKeyHex) && GhalBolFfi.isP2pAvailable)
                    ? () {
                        Navigator.of(context).push<void>(
                          MaterialPageRoute<void>(
                            builder: (_) => ChatScreen(
                              libp2pPeerId: r.publicKeyHex!.trim(),
                              publicKeyHex: r.publicKeyHex,
                              appNamespace: r.appNamespace ?? kGhalBolAndroidLibraryNamespace,
                            ),
                          ),
                        );
                      }
                    : null,
                icon: const Icon(Icons.chat_bubble_outline),
                label: Text(
                  "Open chat",
                  maxLines: 2,
                  overflow: TextOverflow.ellipsis,
                  textAlign: TextAlign.center,
                ),
              ),
            ),
            const SizedBox(height: 8),
            Text(
              "Invitation link includes your identity; dial addresses are added when chat network is running.",
              style: Theme.of(context).textTheme.bodySmall,
            ),
          ],
        ],
      ),
    );
  }
}
