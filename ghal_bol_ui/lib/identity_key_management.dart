import "package:flutter/material.dart";
import "package:flutter/services.dart";
import "package:share_plus/share_plus.dart";

import "ghal_bol_constants.dart";
import "ghal_bol_ffi.dart";
import "identity_setup_copy.dart";

/// Prompt for unlock password; returns trimmed text or null if cancelled.
Future<String?> promptAppPassword(BuildContext context, {required String title}) async {
  final ctrl = TextEditingController();
  final go = await showDialog<bool>(
    context: context,
    barrierDismissible: false,
    builder: (ctx) => AlertDialog(
      title: Text(title),
      content: TextField(
        controller: ctrl,
        obscureText: true,
        autofocus: true,
        decoration: const InputDecoration(
          labelText: "App password",
          border: OutlineInputBorder(),
        ),
        onSubmitted: (_) => Navigator.pop(ctx, true),
      ),
      actions: [
        TextButton(onPressed: () => Navigator.pop(ctx, false), child: const Text("Cancel")),
        FilledButton(onPressed: () => Navigator.pop(ctx, true), child: const Text("Continue")),
      ],
    ),
  );
  final pw = ctrl.text;
  ctrl.dispose();
  if (!context.mounted || go != true || pw.isEmpty) return null;
  return pw;
}

Future<void> showRevealPrivateKeyDialog(BuildContext context) async {
  if (!GhalBolFfi.isIdentityKeyManagementAvailable) {
    _snack(context, "Rebuild native library to enable key reveal.");
    return;
  }
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
            SelectableText(r.secretKeyHex!, style: const TextStyle(fontFamily: "monospace", fontSize: 12)),
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
  final jsonCtrl = TextEditingController();
  final passCtrl = TextEditingController();
  final go = await showDialog<bool>(
    context: context,
    barrierDismissible: false,
    builder: (ctx) => AlertDialog(
      title: const Text("Import keystore backup"),
      content: SingleChildScrollView(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Text(
              "${IdentitySetupCopy.importPrivateKeyWarningBody}\n\n"
              "Paste encrypted Ghal Bol keystore backup JSON. Works when no identity exists on this device "
              "(delete existing identity first).",
              style: TextStyle(color: Theme.of(ctx).colorScheme.onSurface, fontSize: 13),
            ),
            const SizedBox(height: 12),
            TextField(
              controller: jsonCtrl,
              maxLines: 6,
              decoration: const InputDecoration(
                labelText: "Keystore JSON",
                border: OutlineInputBorder(),
              ),
            ),
            const SizedBox(height: 12),
            TextField(
              controller: passCtrl,
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
        TextButton(onPressed: () => Navigator.pop(ctx, false), child: const Text("Cancel")),
        FilledButton(onPressed: () => Navigator.pop(ctx, true), child: const Text("Import")),
      ],
    ),
  );
  final json = jsonCtrl.text.trim();
  final pw = passCtrl.text;
  jsonCtrl.dispose();
  passCtrl.dispose();
  if (!context.mounted || go != true || json.isEmpty || pw.isEmpty) return;

  final r = GhalBolFfi.importKeystoreJson(
    appNamespace: kGhalBolAppNamespace,
    password: pw,
    keystoreJson: json,
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
