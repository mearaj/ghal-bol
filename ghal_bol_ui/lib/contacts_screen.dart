import "package:flutter/material.dart";

import "contact_store.dart";
import "identity_display_name.dart";
import "public_key_hex.dart";
import "saved_contact.dart";

/// List and manage saved 1:1 contacts.
class ContactsScreen extends StatefulWidget {
  const ContactsScreen({super.key, required this.appNamespace});

  final String appNamespace;

  @override
  State<ContactsScreen> createState() => _ContactsScreenState();
}

class _ContactsScreenState extends State<ContactsScreen> {
  List<SavedContact> _contacts = [];

  @override
  void initState() {
    super.initState();
    _load();
    ContactStore.changeCount.addListener(_load);
  }

  @override
  void dispose() {
    ContactStore.changeCount.removeListener(_load);
    super.dispose();
  }

  Future<void> _load() async {
    try {
      final list = await ContactStore.listContacts(widget.appNamespace);
      if (!mounted) return;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!mounted) return;
        setState(() => _contacts = list);
      });
    } catch (e, st) {
      if (!mounted) return;
      debugPrint("ContactsScreen._load failed: $e\n$st");
    }
  }

  Future<void> _addByKeys() async {
    final pkCtrl = TextEditingController();
    final aliasCtrl = TextEditingController();
    final go = await showDialog<bool>(
      context: context,
      builder: (ctx) => AlertDialog(
        title: const Text("Add contact"),
        content: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              const Text(
                "Paste the other person’s identity public key from their invitation "
                "(bare secp256k1 hex or prefixed form such as ed25519:…).",
              ),
              const SizedBox(height: 12),
              TextField(
                controller: pkCtrl,
                decoration: const InputDecoration(
                  labelText: "Public key (hex)",
                  border: OutlineInputBorder(),
                ),
                maxLines: 2,
              ),
              const SizedBox(height: 8),
              TextField(
                controller: aliasCtrl,
                decoration: const InputDecoration(
                  labelText: "Display name (optional)",
                  border: OutlineInputBorder(),
                ),
              ),
            ],
          ),
        ),
        actions: [
          TextButton(onPressed: () => Navigator.pop(ctx, false), child: const Text("Cancel")),
          FilledButton(onPressed: () => Navigator.pop(ctx, true), child: const Text("Add")),
        ],
      ),
    );
    final pk = pkCtrl.text.trim();
    final alias = ghalSanitizePeerAlias(aliasCtrl.text);
    pkCtrl.dispose();
    aliasCtrl.dispose();
    if (go != true || !mounted) return;
    if (!isValidPublicKeyHex(pk)) {
      ScaffoldMessenger.of(context).showSnackBar(
        const SnackBar(content: Text("Invalid identity public key.")),
      );
      return;
    }
    final wire = resolvePublicKeyHex(storedHex: pk) ?? pk;
    await ContactStore.upsertContact(
      appNamespace: widget.appNamespace,
      contact: SavedContact(
        publicKeyHex: wire,
        displayAlias: alias,
        createdAtMs: DateTime.now().millisecondsSinceEpoch,
        updatedAtMs: DateTime.now().millisecondsSinceEpoch,
      ),
    );
    if (!mounted) return;
    ScaffoldMessenger.of(context).showSnackBar(
      const SnackBar(content: Text("Contact saved.")),
    );
  }

  Future<void> _editAlias(SavedContact c) async {
    final raw = await showDialog<String>(
      context: context,
      builder: (ctx) => _EditContactAliasDialog(contact: c),
    );
    if (raw == null || !mounted) return;
    try {
      final updated = await ContactStore.updateDisplayAlias(
        appNamespace: widget.appNamespace,
        contact: c,
        raw: raw,
      );
      if (!mounted) return;
      if (updated == null) {
        ScaffoldMessenger.of(context).showSnackBar(
          const SnackBar(content: Text("Could not save display name.")),
        );
        return;
      }
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!mounted) return;
        setState(() {
          final i = _contacts.indexWhere(
            (x) => x.publicKeyHex == updated.publicKeyHex,
          );
          if (i >= 0) _contacts[i] = updated;
        });
        ScaffoldMessenger.of(context).showSnackBar(
          const SnackBar(content: Text("Display name saved.")),
        );
      });
    } catch (e) {
      if (!mounted) return;
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text("Could not save display name: $e")),
      );
    }
  }

  Future<void> _confirmDelete(SavedContact c) async {
    final ok = await showDialog<bool>(
      context: context,
      builder: (ctx) => AlertDialog(
        title: const Text("Remove contact?"),
        content: Text(
          c.displayAlias?.trim().isNotEmpty == true
              ? "Remove ${c.displayAlias} from your contact list?"
              : "Remove this contact from your list?",
        ),
        actions: [
          TextButton(onPressed: () => Navigator.pop(ctx, false), child: const Text("Cancel")),
          FilledButton(
            onPressed: () => Navigator.pop(ctx, true),
            child: const Text("Remove"),
          ),
        ],
      ),
    );
    if (ok != true || !mounted) return;
    await ContactStore.removeContact(
      appNamespace: widget.appNamespace,
      contact: c,
    );
    await _load();
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        title: const Text("Contacts"),
        actions: [
          IconButton(
            tooltip: "Add by public key",
            onPressed: _addByKeys,
            icon: const Icon(Icons.person_add_alt_1),
          ),
        ],
      ),
      body: _contacts.isEmpty
          ? const Center(child: Text("No contacts yet. Scan a QR or add by public key."))
          : ListView.builder(
              itemCount: _contacts.length,
              itemBuilder: (context, i) {
                final c = _contacts[i];
                final title = ghalBolIdName(
                  publicKeyHex: c.publicKeyHex,
                  customAlias: c.displayAlias,
                );
                return ListTile(
                  title: Text(title),
                  subtitle: Text(
                    c.hasPublicKey ? "keys ready" : "missing public key",
                  ),
                  onTap: () => _editAlias(c),
                  trailing: Row(
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      IconButton(
                        tooltip: "Edit display name",
                        icon: const Icon(Icons.edit_outlined),
                        onPressed: () => _editAlias(c),
                      ),
                      IconButton(
                        tooltip: "Remove contact",
                        icon: const Icon(Icons.delete_outline),
                        onPressed: () => _confirmDelete(c),
                      ),
                    ],
                  ),
                );
              },
            ),
    );
  }
}

/// Edit dialog — controller owned by [State] so dispose never races route pop.
class _EditContactAliasDialog extends StatefulWidget {
  const _EditContactAliasDialog({required this.contact});

  final SavedContact contact;

  @override
  State<_EditContactAliasDialog> createState() => _EditContactAliasDialogState();
}

class _EditContactAliasDialogState extends State<_EditContactAliasDialog> {
  late final TextEditingController _aliasCtrl;

  @override
  void initState() {
    super.initState();
    _aliasCtrl = TextEditingController(text: widget.contact.displayAlias ?? "");
  }

  @override
  void dispose() {
    _aliasCtrl.dispose();
    super.dispose();
  }

  void _save() => Navigator.pop(context, _aliasCtrl.text);

  @override
  Widget build(BuildContext context) {
    final c = widget.contact;
    return AlertDialog(
      title: const Text("Edit display name"),
      content: SingleChildScrollView(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Text(
              ghalBolIdName(publicKeyHex: c.publicKeyHex, customAlias: null),
              style: Theme.of(context).textTheme.bodySmall,
            ),
            const SizedBox(height: 12),
            TextField(
              controller: _aliasCtrl,
              autofocus: true,
              textCapitalization: TextCapitalization.sentences,
              decoration: const InputDecoration(
                labelText: "Display name",
                hintText: "Leave blank for default",
                border: OutlineInputBorder(),
              ),
              textInputAction: TextInputAction.done,
              onSubmitted: (_) => _save(),
            ),
          ],
        ),
      ),
      actions: [
        TextButton(onPressed: () => Navigator.pop(context), child: const Text("Cancel")),
        FilledButton(onPressed: _save, child: const Text("Save")),
      ],
    );
  }
}
