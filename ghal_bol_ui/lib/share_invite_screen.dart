import "package:flutter/material.dart";
import "package:flutter/services.dart";
import "package:qr_flutter/qr_flutter.dart";
import "package:share_plus/share_plus.dart";

/// Full-screen host invitation — QR encodes format-2 connect invite (public key only).
class ShareInviteScreen extends StatefulWidget {
  const ShareInviteScreen({
    super.key,
    required this.readInviteUri,
    required this.readListenReady,
    required this.onParentRefresh,
  });

  final String? Function() readInviteUri;
  final bool Function() readListenReady;
  final VoidCallback onParentRefresh;

  @override
  State<ShareInviteScreen> createState() => _ShareInviteScreenState();
}

class _ShareInviteScreenState extends State<ShareInviteScreen> {
  void _rebuild() {
    widget.onParentRefresh();
    if (mounted) setState(() {});
  }

  @override
  Widget build(BuildContext context) {
    final uri = widget.readInviteUri();
    final listenReady = widget.readListenReady();

    return Scaffold(
      appBar: AppBar(
        title: const Text("Your QR invitation"),
      ),
      body: SafeArea(
        child: ListView(
          padding: const EdgeInsets.fromLTRB(20, 12, 20, 28),
          children: [
            if (uri != null) ...[
              Builder(
                builder: (_) {
                  final qr = QrValidator.validate(data: uri);
                  if (qr.status == QrValidationStatus.valid) {
                    return Center(
                      child: DecoratedBox(
                        decoration: BoxDecoration(
                          color: Colors.white,
                          borderRadius: BorderRadius.circular(12),
                        ),
                        child: QrImageView(
                          data: uri,
                          size: 280,
                          padding: const EdgeInsets.all(14),
                        ),
                      ),
                    );
                  }
                  return Padding(
                    padding: const EdgeInsets.symmetric(vertical: 12),
                    child: Text(
                      "This link is too long for a QR (${uri.length} characters). "
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
                      onPressed: _rebuild,
                      icon: const Icon(Icons.refresh),
                      label: const Text("Refresh now"),
                    ),
                  ],
                ),
              ),
            Text(
              "QR encodes a link to your public key on ghalbol.com (no PeerId, no IP addresses). "
              "They tap Join → Scan QR (or paste the link). The app looks up your endpoints on the coordination server.",
              style: Theme.of(context).textTheme.bodyLarge,
              textAlign: TextAlign.center,
            ),
            const SizedBox(height: 12),
            // Removed: redundant "P2P still starting" banner (spinner + button already cover it).
            if (uri != null) ...[
              const SizedBox(height: 20),
              SelectableText(uri, style: const TextStyle(fontSize: 11)),
              const SizedBox(height: 16),
              FilledButton.icon(
                onPressed: () async {
                  await Clipboard.setData(ClipboardData(text: uri));
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
                    ShareParams(text: uri, subject: "Ghal Bol chat invitation"),
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
