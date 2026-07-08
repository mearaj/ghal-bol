import "dart:async";

import "package:flutter/material.dart";
import "package:flutter/services.dart";
import "package:qr_flutter/qr_flutter.dart";
import "package:share_plus/share_plus.dart";

import "package:ghal_bol_ui/identity_alias_store.dart";
import "package:ghal_bol_ui/invite_uri_builder.dart";
import "package:ghal_bol_ui/public_key_hex.dart";

/// Full-screen host invitation — one QR for the HTTPS invite (Android / desktop).
class ShareInviteScreen extends StatefulWidget {
  const ShareInviteScreen({
    super.key,
    required this.publicKeyHex,
    this.identityWire,
    required this.appNamespace,
    required this.readListenReady,
    required this.onParentRefresh,
  });

  final String publicKeyHex;
  final String? identityWire;
  final String appNamespace;
  final bool Function() readListenReady;
  final VoidCallback onParentRefresh;

  @override
  State<ShareInviteScreen> createState() => _ShareInviteScreenState();
}

class _ShareInviteScreenState extends State<ShareInviteScreen> {
  String? _uri;
  bool _loadingUri = true;

  @override
  void initState() {
    super.initState();
    unawaited(_reloadUri());
  }

  /// Always read alias from native store so QR matches Copy / Share after Save.
  Future<void> _reloadUri() async {
    setState(() => _loadingUri = true);
    widget.onParentRefresh();
    final wire = identityWireFromSession(
      identityWire: widget.identityWire,
      publicKeyHex: widget.publicKeyHex,
    );
    String? uri;
    if (wire != null && isValidPublicKeyHex(wire)) {
      final alias = await IdentityAliasStore.read(
        appNamespace: widget.appNamespace,
        publicKeyHex: widget.publicKeyHex.trim(),
      );
      uri = buildGhalBolInviteUri(
        publicKeyHex: widget.publicKeyHex.trim(),
        identityWire: wire,
        peerAlias: alias,
      );
    }
    if (!mounted) return;
    setState(() {
      _uri = uri;
      _loadingUri = false;
    });
  }

  @override
  Widget build(BuildContext context) {
    final uri = _uri;
    final listenReady = widget.readListenReady();
    final waiting = _loadingUri || uri == null;
    final inviteUri = uri;

    return Scaffold(
      appBar: AppBar(
        title: const Text("Your QR invitation"),
      ),
      body: SafeArea(
        child: ListView(
          padding: const EdgeInsets.fromLTRB(20, 12, 20, 28),
          children: [
            if (!waiting && inviteUri != null) ...[
              Builder(
                builder: (_) {
                  final qr = QrValidator.validate(data: inviteUri);
                  if (qr.status == QrValidationStatus.valid) {
                    return Center(
                      child: DecoratedBox(
                        decoration: BoxDecoration(
                          color: Colors.white,
                          borderRadius: BorderRadius.circular(12),
                        ),
                        child: QrImageView(
                          data: inviteUri,
                          size: 280,
                          padding: const EdgeInsets.all(14),
                        ),
                      ),
                    );
                  }
                  return Padding(
                    padding: const EdgeInsets.symmetric(vertical: 12),
                    child: Text(
                      "This link is too long for a QR (${inviteUri.length} characters). "
                      "Use Copy link below — the other person can paste it in Join.",
                      style: TextStyle(color: Theme.of(context).colorScheme.error),
                      textAlign: TextAlign.center,
                    ),
                  );
                },
              ),
              const SizedBox(height: 16),
            ] else
              Padding(
                padding: const EdgeInsets.symmetric(vertical: 24),
                child: Column(
                  children: [
                    const CircularProgressIndicator(),
                    const SizedBox(height: 16),
                    Text(
                      listenReady
                          ? "Building invitation…"
                          : "Starting P2P… QR will appear when the node is ready.",
                      textAlign: TextAlign.center,
                      style: Theme.of(context).textTheme.bodyMedium,
                    ),
                    const SizedBox(height: 12),
                    FilledButton.tonalIcon(
                      onPressed: () => unawaited(_reloadUri()),
                      icon: const Icon(Icons.refresh),
                      label: const Text("Refresh now"),
                    ),
                  ],
                ),
              ),
            Text(
              "QR encodes your https://ghalbol.com invite (public key and saved display name). "
              "They tap Join → Scan QR (or paste the link).",
              style: Theme.of(context).textTheme.bodyLarge,
              textAlign: TextAlign.center,
            ),
            if (!waiting && inviteUri != null) ...[
              const SizedBox(height: 20),
              SelectableText(inviteUri, style: const TextStyle(fontSize: 11)),
              const SizedBox(height: 16),
              FilledButton.icon(
                onPressed: () async {
                  await Clipboard.setData(ClipboardData(text: inviteUri));
                  if (context.mounted) {
                    ScaffoldMessenger.of(context).showSnackBar(
                      const SnackBar(content: Text("Invitation copied")),
                    );
                  }
                },
                icon: const Icon(Icons.copy),
                label: const Text("Copy link"),
              ),
              const SizedBox(height: 8),
              OutlinedButton.icon(
                onPressed: () async {
                  await SharePlus.instance.share(
                    ShareParams(text: inviteUri, subject: "Ghal Bol chat invitation"),
                  );
                },
                icon: const Icon(Icons.share_outlined),
                label: const Text("Share link…"),
              ),
            ],
          ],
        ),
      ),
    );
  }
}
