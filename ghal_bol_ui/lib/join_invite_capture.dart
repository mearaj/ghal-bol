import "package:flutter/foundation.dart" show kIsWeb;
import "package:flutter/material.dart";

import "invite_paste_dialog.dart";
import "invite_scan_screen.dart";

export "invite_paste_dialog.dart" show showPasteInviteLinkDialog;

/// Scan QR or paste — hub app bar uses this; does not route through chat P2P gate.
Future<String?> captureJoinInviteUri(BuildContext context) async {
  if (kIsWeb) {
    final pasted = await showPasteInviteLinkDialog(context);
    if (pasted == null || pasted.isEmpty) return null;
    return InviteScanScreen.extractInviteUri(pasted) ?? pasted.trim();
  }
  final mode = await showModalBottomSheet<String>(
    context: context,
    showDragHandle: true,
    builder: (ctx) => SafeArea(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          ListTile(
            leading: const Icon(Icons.qr_code_scanner),
            title: const Text("Scan QR code"),
            onTap: () => Navigator.pop(ctx, "scan"),
          ),
          ListTile(
            leading: const Icon(Icons.link),
            title: const Text("Paste invitation link"),
            onTap: () => Navigator.pop(ctx, "paste"),
          ),
        ],
      ),
    ),
  );
  if (!context.mounted || mode == null) return null;
  if (mode == "paste") {
    final pasted = await showPasteInviteLinkDialog(context);
    if (pasted == null || pasted.isEmpty) return null;
    return InviteScanScreen.extractInviteUri(pasted) ?? pasted.trim();
  }
  final raw = await Navigator.of(context).push<String>(
    MaterialPageRoute(builder: (_) => const InviteScanScreen()),
  );
  if (raw == null || raw.isEmpty) return null;
  return InviteScanScreen.extractInviteUri(raw) ?? raw.trim();
}
