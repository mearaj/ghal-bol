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
    final list = await ContactStore.listContacts(widget.appNamespace);
    if (!mounted) return;
    setState(() => _contacts = list);
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
                "Paste the other person’s secp256k1 public key (66 hex chars) from their invitation.",
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
        const SnackBar(content: Text("Public key must be 66 hex characters.")),
      );
      return;
    }
    await ContactStore.upsertContact(
      appNamespace: widget.appNamespace,
      contact: SavedContact(
        publicKeyHex: pk.toLowerCase(),
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
                  trailing: IconButton(
                    icon: const Icon(Icons.delete_outline),
                    onPressed: () => _confirmDelete(c),
                  ),
                );
              },
            ),
    );
  }
}
