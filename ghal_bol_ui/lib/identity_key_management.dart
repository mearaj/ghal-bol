import "package:flutter/material.dart";
import "package:flutter/services.dart";
import "package:share_plus/share_plus.dart";

import "ghal_bol_constants.dart";
import "ghal_bol_ffi.dart";
import "identity_setup_copy.dart";
import "session_credentials.dart";

/// Dismiss focus before stacking dialogs over the Identity tab.
void dismissTextSelectionForDialog(BuildContext context) {
  FocusManager.instance.primaryFocus?.unfocus();
}

/// Prompt for unlock password; returns trimmed text or null if cancelled.
Future<String?> promptAppPassword(BuildContext context, {required String title}) {
  dismissTextSelectionForDialog(context);
  return showDialog<String?>(
    context: context,
    barrierDismissible: false,
    builder: (ctx) => _AppPasswordDialog(title: title),
  );
}

/// Controller owned by [State] — never dispose outside the dialog route (see DESIGN.md).
class _AppPasswordDialog extends StatefulWidget {
  const _AppPasswordDialog({required this.title});

  final String title;

  @override
  State<_AppPasswordDialog> createState() => _AppPasswordDialogState();
}

class _AppPasswordDialogState extends State<_AppPasswordDialog> {
  final _ctrl = TextEditingController();

  @override
  void dispose() {
    _ctrl.dispose();
    super.dispose();
  }

  void _submit() {
    final pw = _ctrl.text.trim();
    if (pw.isEmpty) return;
    Navigator.pop(context, pw);
  }

  @override
  Widget build(BuildContext context) {
    return AlertDialog(
      title: Text(widget.title),
      content: TextField(
        controller: _ctrl,
        obscureText: true,
        autofocus: true,
        decoration: const InputDecoration(
          labelText: "App password",
          border: OutlineInputBorder(),
        ),
        onSubmitted: (_) => _submit(),
      ),
      actions: [
        TextButton(onPressed: () => Navigator.pop(context), child: const Text("Cancel")),
        FilledButton(onPressed: _submit, child: const Text("Continue")),
      ],
    );
  }
}

class _ImportKeystoreBackupResult {
  const _ImportKeystoreBackupResult({required this.json, required this.password});

  final String json;
  final String password;
}

class _ImportKeystoreBackupDialog extends StatefulWidget {
  const _ImportKeystoreBackupDialog();

  @override
  State<_ImportKeystoreBackupDialog> createState() => _ImportKeystoreBackupDialogState();
}

class _ImportKeystoreBackupDialogState extends State<_ImportKeystoreBackupDialog> {
  final _jsonCtrl = TextEditingController();
  final _passCtrl = TextEditingController();

  @override
  void dispose() {
    _jsonCtrl.dispose();
    _passCtrl.dispose();
    super.dispose();
  }

  void _submit() {
    final json = _jsonCtrl.text.trim();
    final pw = _passCtrl.text;
    if (json.isEmpty || pw.isEmpty) return;
    Navigator.pop(context, _ImportKeystoreBackupResult(json: json, password: pw));
  }

  @override
  Widget build(BuildContext context) {
    return AlertDialog(
      title: const Text("Import keystore backup"),
      content: SingleChildScrollView(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Text(
              "${IdentitySetupCopy.importPrivateKeyWarningBody}\n\n"
              "Paste encrypted Ghal Bol keystore backup JSON. Works when no identity exists on this device "
              "(delete existing identity first).",
              style: TextStyle(color: Theme.of(context).colorScheme.onSurface, fontSize: 13),
            ),
            const SizedBox(height: 12),
            TextField(
              controller: _jsonCtrl,
              maxLines: 6,
              decoration: const InputDecoration(
                labelText: "Keystore JSON",
                border: OutlineInputBorder(),
              ),
            ),
            const SizedBox(height: 12),
            TextField(
              controller: _passCtrl,
              obscureText: true,
              decoration: const InputDecoration(
                labelText: "App password for this backup",
                border: OutlineInputBorder(),
              ),
            ),
          ],
        ),
      ),
      actions: [
        TextButton(onPressed: () => Navigator.pop(context), child: const Text("Cancel")),
        FilledButton(onPressed: _submit, child: const Text("Import")),
      ],
    );
  }
}

/// Delete-identity confirmation with password — same controller lifecycle as [_AppPasswordDialog].
Future<String?> promptDeleteIdentityPassword(
  BuildContext context, {
  required String body,
}) {
  dismissTextSelectionForDialog(context);
  return showDialog<String?>(
    context: context,
    barrierDismissible: false,
    builder: (ctx) => _DeleteIdentityPasswordDialog(body: body),
  );
}

class _DeleteIdentityPasswordDialog extends StatefulWidget {
  const _DeleteIdentityPasswordDialog({required this.body});

  final String body;

  @override
  State<_DeleteIdentityPasswordDialog> createState() => _DeleteIdentityPasswordDialogState();
}

class _DeleteIdentityPasswordDialogState extends State<_DeleteIdentityPasswordDialog> {
  final _ctrl = TextEditingController();

  @override
  void dispose() {
    _ctrl.dispose();
    super.dispose();
  }

  void _submit() {
    final pw = _ctrl.text.trim();
    if (pw.isEmpty) return;
    Navigator.pop(context, pw);
  }

  @override
  Widget build(BuildContext context) {
    final error = Theme.of(context).colorScheme.error;
    return AlertDialog(
      title: const Text("Delete identity?"),
      content: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Text(widget.body),
          const SizedBox(height: 12),
          TextField(
            controller: _ctrl,
            obscureText: true,
            decoration: const InputDecoration(
              labelText: "Password",
              border: OutlineInputBorder(),
            ),
            onSubmitted: (_) => _submit(),
          ),
        ],
      ),
      actions: [
        TextButton(onPressed: () => Navigator.pop(context), child: const Text("Cancel")),
        FilledButton(
          style: FilledButton.styleFrom(backgroundColor: error),
          onPressed: _submit,
          child: const Text("Delete"),
        ),
      ],
    );
  }
}

Future<void> showRevealPrivateKeyDialog(BuildContext context) async {
  if (!GhalBolFfi.isIdentityKeyManagementAvailable) {
    _snack(context, "Rebuild native library to enable key reveal.");
    return;
  }
  dismissTextSelectionForDialog(context);
  final proceed = await showDialog<bool>(
    context: context,
    builder: (ctx) => AlertDialog(
      title: const Text("Show private key?"),
      content: const Text(IdentitySetupCopy.revealKeyWarning),
      actions: [
        TextButton(onPressed: () => Navigator.pop(ctx, false), child: const Text("Cancel")),
        FilledButton(onPressed: () => Navigator.pop(ctx, true), child: const Text("Continue")),
      ],
    ),
  );
  if (!context.mounted || proceed != true) return;

  final pw = await promptAppPassword(context, title: "App password required");
  if (pw == null) return;
  final r = GhalBolFfi.revealSecretKeyHex(
    appNamespace: kGhalBolAppNamespace,
    password: pw,
  );
  if (!context.mounted) return;
  if (!r.ok || r.secretKeyHex == null) {
    _snack(context, r.error ?? "Wrong password or reveal failed");
    return;
  }
  final algo = r.identityAlgorithm?.trim().isNotEmpty == true
      ? r.identityAlgorithm!.trim()
      : "secp256k1";
  await showDialog<void>(
    context: context,
    builder: (ctx) => AlertDialog(
      title: const Text("Private key (secret)"),
      content: SingleChildScrollView(
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          mainAxisSize: MainAxisSize.min,
          children: [
            Text(
              "Your Ghal Bol messaging key ($algo). Do not share this hex — "
              "anyone with it controls your chat identity. "
              "If this key is also used for cryptocurrency, treat exposure as a wallet risk.",
              style: Theme.of(ctx).textTheme.bodySmall,
            ),
            const SizedBox(height: 12),
            Text(
              r.secretKeyHex!,
              style: const TextStyle(fontFamily: "monospace", fontSize: 12),
            ),
          ],
        ),
      ),
      actions: [
        TextButton(
          onPressed: () {
            Clipboard.setData(ClipboardData(text: r.secretKeyHex!));
            ScaffoldMessenger.of(ctx).showSnackBar(const SnackBar(content: Text("Private key copied")));
          },
          child: const Text("Copy"),
        ),
        FilledButton(onPressed: () => Navigator.pop(ctx), child: const Text("Close")),
      ],
    ),
  );
}

Future<void> exportKeystoreBackup(BuildContext context) async {
  if (!GhalBolFfi.isIdentityKeyManagementAvailable) {
    _snack(context, "Rebuild native library to enable export.");
    return;
  }
  dismissTextSelectionForDialog(context);
  final proceed = await showDialog<bool>(
    context: context,
    builder: (ctx) => AlertDialog(
      title: const Text("Export identity backup?"),
      content: const Text(
        "Anyone with this encrypted backup and your app password can fully control "
        "your communication identity. Store it only somewhere you trust.",
      ),
      actions: [
        TextButton(onPressed: () => Navigator.pop(ctx, false), child: const Text("Cancel")),
        FilledButton(onPressed: () => Navigator.pop(ctx, true), child: const Text("Export")),
      ],
    ),
  );
  if (!context.mounted || proceed != true) return;

  final r = GhalBolFfi.exportKeystoreJson(appNamespace: kGhalBolAppNamespace);
  if (!context.mounted) return;
  if (!r.ok || r.keystoreJson == null) {
    _snack(context, r.error ?? "Export failed");
    return;
  }
  await Clipboard.setData(ClipboardData(text: r.keystoreJson!));
  await SharePlus.instance.share(
    ShareParams(text: r.keystoreJson!, subject: "Ghal Bol keystore backup"),
  );
  if (!context.mounted) return;
  _snack(
    context,
    "Backup exported. Anyone with this file and your password controls your identity.",
  );
}

/// Change the app password that encrypts the on-disk keystore. Verifies the
/// current password, then re-encrypts the same identity under a new one. After
/// success, reminds the user that older exported backups still need the old
/// password (they should re-export).
Future<void> showChangePasswordDialog(BuildContext context) async {
  if (!GhalBolFfi.isChangePasswordAvailable) {
    _snack(context, "Rebuild native library to enable password change.");
    return;
  }
  dismissTextSelectionForDialog(context);
  final changed = await showDialog<bool>(
    context: context,
    barrierDismissible: false,
    builder: (ctx) => const _ChangePasswordDialog(),
  );
  if (changed == true && context.mounted) {
    final reExport = await showDialog<bool>(
      context: context,
      barrierDismissible: false,
      builder: (ctx) => AlertDialog(
        title: const Text("Password changed"),
        content: const Text(
          "Your identity is now encrypted with the new password. Any backup you "
          "exported earlier still needs the OLD password. Export a fresh backup now "
          "so your latest backup matches the new password.",
        ),
        actions: [
          TextButton(onPressed: () => Navigator.pop(ctx, false), child: const Text("Later")),
          FilledButton(onPressed: () => Navigator.pop(ctx, true), child: const Text("Export backup")),
        ],
      ),
    );
    if (reExport == true && context.mounted) {
      await exportKeystoreBackup(context);
    }
  }
}

class _ChangePasswordDialog extends StatefulWidget {
  const _ChangePasswordDialog();

  @override
  State<_ChangePasswordDialog> createState() => _ChangePasswordDialogState();
}

class _ChangePasswordDialogState extends State<_ChangePasswordDialog> {
  final _currentCtrl = TextEditingController();
  final _newCtrl = TextEditingController();
  final _confirmCtrl = TextEditingController();
  bool _busy = false;
  String? _error;

  @override
  void dispose() {
    _currentCtrl.dispose();
    _newCtrl.dispose();
    _confirmCtrl.dispose();
    super.dispose();
  }

  Future<void> _submit() async {
    final current = _currentCtrl.text;
    final next = _newCtrl.text;
    final confirm = _confirmCtrl.text;
    if (current.isEmpty || next.isEmpty) {
      setState(() => _error = "Enter your current and new password.");
      return;
    }
    if (next != confirm) {
      setState(() => _error = "New passwords do not match.");
      return;
    }
    if (next == current) {
      setState(() => _error = "New password must differ from the current one.");
      return;
    }
    setState(() {
      _busy = true;
      _error = null;
    });
    final r = GhalBolFfi.changePassword(
      appNamespace: kGhalBolAppNamespace,
      oldPassword: current,
      newPassword: next,
    );
    if (!mounted) return;
    if (!r.ok) {
      setState(() {
        _busy = false;
        _error = r.error ?? "Wrong current password or change failed.";
      });
      return;
    }
    // Keep the cached credential (used to re-unlock the P2P daemon after restart)
    // in sync with the new password. The daemon's live session stays unlocked.
    SessionCredentials.store(appNamespace: kGhalBolAppNamespace, password: next);
    Navigator.pop(context, true);
  }

  @override
  Widget build(BuildContext context) {
    return AlertDialog(
      title: const Text("Change app password"),
      content: SingleChildScrollView(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            const Text(
              "This changes the password that encrypts your identity on this device. "
              "It does not change your identity or public key.",
            ),
            const SizedBox(height: 12),
            TextField(
              controller: _currentCtrl,
              obscureText: true,
              decoration: const InputDecoration(
                labelText: "Current password",
                border: OutlineInputBorder(),
              ),
            ),
            const SizedBox(height: 12),
            TextField(
              controller: _newCtrl,
              obscureText: true,
              decoration: const InputDecoration(
                labelText: "New password",
                border: OutlineInputBorder(),
              ),
            ),
            const SizedBox(height: 12),
            TextField(
              controller: _confirmCtrl,
              obscureText: true,
              decoration: const InputDecoration(
                labelText: "Confirm new password",
                border: OutlineInputBorder(),
              ),
              onSubmitted: (_) => _busy ? null : _submit(),
            ),
            if (_error != null) ...[
              const SizedBox(height: 12),
              Text(
                _error!,
                style: TextStyle(color: Theme.of(context).colorScheme.error),
              ),
            ],
          ],
        ),
      ),
      actions: [
        TextButton(
          onPressed: _busy ? null : () => Navigator.pop(context, false),
          child: const Text("Cancel"),
        ),
        FilledButton(
          onPressed: _busy ? null : _submit,
          child: Text(_busy ? "…" : "Change"),
        ),
      ],
    );
  }
}

/// Blocking, one-time backup step shown only after the app **generates** a new
/// identity (not import). The user must export the encrypted keystore and
/// confirm they saved it before entering the app. Loss of a device-generated
/// key with no backup is unrecoverable.
///
/// No-op (returns immediately) when native key management is unavailable — we
/// cannot force an export the native layer cannot produce.
Future<void> showMandatoryKeystoreBackupDialog(BuildContext context) async {
  if (!GhalBolFfi.isIdentityKeyManagementAvailable) return;
  dismissTextSelectionForDialog(context);
  await showDialog<void>(
    context: context,
    barrierDismissible: false,
    builder: (ctx) => const _MandatoryBackupDialog(),
  );
}

class _MandatoryBackupDialog extends StatefulWidget {
  const _MandatoryBackupDialog();

  @override
  State<_MandatoryBackupDialog> createState() => _MandatoryBackupDialogState();
}

class _MandatoryBackupDialogState extends State<_MandatoryBackupDialog> {
  bool _exported = false;
  bool _confirmedSaved = false;
  bool _busy = false;

  Future<void> _export() async {
    setState(() => _busy = true);
    try {
      final r = GhalBolFfi.exportKeystoreJson(appNamespace: kGhalBolAppNamespace);
      if (!mounted) return;
      if (!r.ok || r.keystoreJson == null) {
        _snack(context, r.error ?? "Export failed");
        return;
      }
      await Clipboard.setData(ClipboardData(text: r.keystoreJson!));
      await SharePlus.instance.share(
        ShareParams(text: r.keystoreJson!, subject: "Ghal Bol keystore backup"),
      );
      if (!mounted) return;
      setState(() => _exported = true);
      _snack(context, "Backup exported. Store it somewhere only you can reach.");
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final canContinue = _exported && _confirmedSaved && !_busy;
    return PopScope(
      canPop: false,
      child: AlertDialog(
        title: const Text("Back up your identity now"),
        content: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Text(
                "Ghal Bol just generated your identity on this device. There is no "
                "cloud copy and no way to recover it if this device is lost, reset, "
                "or the app is deleted.\n\n"
                "Export the encrypted backup and keep it somewhere safe. It is "
                "encrypted with the app password you just chose — you will need "
                "that same password to restore it.",
                style: theme.textTheme.bodyMedium,
              ),
              const SizedBox(height: 16),
              FilledButton.icon(
                onPressed: _busy ? null : _export,
                icon: const Icon(Icons.download_outlined, size: 18),
                label: Text(_exported ? "Export backup again" : "Export encrypted backup"),
              ),
              if (_exported) ...[
                const SizedBox(height: 8),
                Row(
                  children: [
                    Icon(Icons.check_circle_outline,
                        size: 18, color: theme.colorScheme.primary),
                    const SizedBox(width: 6),
                    Expanded(
                      child: Text(
                        "Backup exported and copied to clipboard.",
                        style: theme.textTheme.bodySmall,
                      ),
                    ),
                  ],
                ),
              ],
              const SizedBox(height: 8),
              CheckboxListTile(
                contentPadding: EdgeInsets.zero,
                controlAffinity: ListTileControlAffinity.leading,
                value: _confirmedSaved,
                onChanged: (!_exported || _busy)
                    ? null
                    : (v) => setState(() => _confirmedSaved = v ?? false),
                title: Text(
                  "I have saved my encrypted backup and my app password.",
                  style: theme.textTheme.bodyMedium,
                ),
              ),
            ],
          ),
        ),
        actions: [
          FilledButton(
            onPressed: canContinue ? () => Navigator.pop(context) : null,
            child: const Text("Continue"),
          ),
        ],
      ),
    );
  }
}

Future<void> importKeystoreBackup(BuildContext context, {required void Function() onImported}) async {
  if (!GhalBolFfi.isIdentityKeyManagementAvailable) {
    _snack(context, "Rebuild native library to enable import.");
    return;
  }
  dismissTextSelectionForDialog(context);
  final payload = await showDialog<_ImportKeystoreBackupResult>(
    context: context,
    barrierDismissible: false,
    builder: (ctx) => const _ImportKeystoreBackupDialog(),
  );
  if (!context.mounted || payload == null) return;

  final r = GhalBolFfi.importKeystoreJson(
    appNamespace: kGhalBolAppNamespace,
    password: payload.password,
    keystoreJson: payload.json,
  );
  if (!context.mounted) return;
  if (!r.ok) {
    GhalBolFfi.resetFirstTimeIdentity(appNamespace: kGhalBolAppNamespace);
    GhalBolFfi.lock();
    _snack(context, "${r.error ?? "Import failed"}${IdentitySetupCopy.firstTimeRetryHint}");
    return;
  }
  onImported();
  _snack(context, "Identity imported. Enter your app password to unlock.");
}

void _snack(BuildContext context, String msg) {
  ScaffoldMessenger.of(context).showSnackBar(SnackBar(content: Text(msg)));
}
