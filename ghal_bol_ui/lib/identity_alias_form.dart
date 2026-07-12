import "package:flutter/material.dart";

import "ghal_bol_ffi.dart";
import "identity_alias_store.dart";
import "identity_display_name.dart";

/// Lets the owner set a display alias (persisted by **`ghal_bol`**); empty reverts to the hex `abcd..wxyz` default.
class IdentityAliasForm extends StatefulWidget {
  const IdentityAliasForm({
    super.key,
    required this.appNamespace,
    required this.publicKeyHex,
    required this.onSaved,
  });

  final String appNamespace;
  final String publicKeyHex;
  final void Function(String? storedSanitized) onSaved;

  @override
  State<IdentityAliasForm> createState() => _IdentityAliasFormState();
}

class _IdentityAliasFormState extends State<IdentityAliasForm> {
  late final TextEditingController _ctrl = TextEditingController();
  bool _loading = true;

  @override
  void initState() {
    super.initState();
    _load();
  }

  Future<void> _load() async {
    if (!GhalBolFfi.isPeerDisplayAliasAvailable) {
      if (mounted) setState(() => _loading = false);
      return;
    }
    final v = await IdentityAliasStore.read(
      appNamespace: widget.appNamespace,
      publicKeyHex: widget.publicKeyHex,
    );
    if (!mounted) return;
    _ctrl.text = v ?? "";
    setState(() => _loading = false);
  }

  @override
  void dispose() {
    _ctrl.dispose();
    super.dispose();
  }

  Future<void> _save() async {
    await IdentityAliasStore.write(
      appNamespace: widget.appNamespace,
      publicKeyHex: widget.publicKeyHex,
      raw: _ctrl.text,
    );
    final v = await IdentityAliasStore.read(
      appNamespace: widget.appNamespace,
      publicKeyHex: widget.publicKeyHex,
    );
    if (!mounted) return;
    widget.onSaved(v);
    ScaffoldMessenger.of(context).showSnackBar(
      const SnackBar(content: Text("Display name saved")),
    );
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final def = ghalBolIdName(
      publicKeyHex: widget.publicKeyHex,
      customAlias: null,
    );
    if (_loading) {
      return const Padding(
        padding: EdgeInsets.symmetric(vertical: 12),
        child: Center(child: SizedBox(width: 22, height: 22, child: CircularProgressIndicator(strokeWidth: 2))),
      );
    }
    if (!GhalBolFfi.isPeerDisplayAliasAvailable) {
      return Text(
        "Display name needs a newer native build (`ghal_bol_core_ffi_peer_display_alias_*`).",
        style: theme.textTheme.bodySmall?.copyWith(color: theme.colorScheme.outline),
      );
    }
    return Material(
      type: MaterialType.transparency,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Text("Display name", style: theme.textTheme.titleSmall),
          const SizedBox(height: 4),
          Text(
            "Optional. Leave blank to use the default ($def). Stored by Ghal Bol native; included in invitations only when set.",
            style: theme.textTheme.bodySmall?.copyWith(color: theme.colorScheme.outline),
          ),
          const SizedBox(height: 10),
          TextField(
            controller: _ctrl,
            textCapitalization: TextCapitalization.sentences,
            decoration: const InputDecoration(
              border: OutlineInputBorder(),
              hintText: "e.g. Kitchen tablet",
            ),
            onSubmitted: (_) => _save(),
          ),
          const SizedBox(height: 10),
          FilledButton(onPressed: _save, child: const Text("Save display name")),
        ],
      ),
    );
  }
}
