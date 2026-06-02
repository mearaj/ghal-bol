import "dart:async" show unawaited;

import "package:flutter/material.dart";

import "contact_store.dart";
import "identity_display_name.dart";
import "saved_contact.dart";

/// Contacts with `is_blocked: true` for this app namespace.
class BlockedPeersScreen extends StatefulWidget {
  const BlockedPeersScreen({super.key, required this.appNamespace});

  final String appNamespace;

  @override
  State<BlockedPeersScreen> createState() => _BlockedPeersScreenState();
}

class _BlockedPeersScreenState extends State<BlockedPeersScreen> {
  List<SavedContact> _blocked = [];
  bool _loading = true;

  @override
  void initState() {
    super.initState();
    ContactStore.changeCount.addListener(_load);
    unawaited(_load());
  }

  @override
  void dispose() {
    ContactStore.changeCount.removeListener(_load);
    super.dispose();
  }

  Future<void> _load() async {
    final all = await ContactStore.listContacts(widget.appNamespace);
    if (!mounted) return;
    setState(() {
      _blocked = all.where((c) => c.isBlocked).toList();
      _loading = false;
    });
  }

  Future<void> _unblock(SavedContact c) async {
    await ContactStore.setTrust(
      appNamespace: widget.appNamespace,
      publicKeyHex: c.publicKeyHex,
      isBlocked: false,
    );
    if (!mounted) return;
    ScaffoldMessenger.of(context).showSnackBar(
      const SnackBar(content: Text("Contact unblocked.")),
    );
    await _load();
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(title: const Text("Blocked contacts")),
      body: _loading
          ? const Center(child: CircularProgressIndicator())
          : _blocked.isEmpty
              ? const Center(
                  child: Padding(
                    padding: EdgeInsets.all(24),
                    child: Text(
                      "No blocked contacts. Block someone from the unknown banner in a chat room.",
                      textAlign: TextAlign.center,
                    ),
                  ),
                )
              : ListView.builder(
                  itemCount: _blocked.length,
                  itemBuilder: (ctx, i) {
                    final c = _blocked[i];
                    final label = ghalBolIdName(
                      publicKeyHex: c.publicKeyHex,
                      customAlias: c.displayAlias,
                    );
                    return ListTile(
                      title: Text(label),
                      subtitle: Text(c.publicKeyHex, maxLines: 1, overflow: TextOverflow.ellipsis),
                      trailing: TextButton(
                        onPressed: () => unawaited(_unblock(c)),
                        child: const Text("Unblock"),
                      ),
                    );
                  },
                ),
    );
  }
}
