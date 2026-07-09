import "package:flutter/material.dart";
import "package:flutter/services.dart";
import "package:share_plus/share_plus.dart";

import "ghal_bol_constants.dart";
import "ghal_bol_ffi.dart";
import "identity_setup_copy.dart";

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
