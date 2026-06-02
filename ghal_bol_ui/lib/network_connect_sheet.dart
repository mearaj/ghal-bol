import "package:flutter/material.dart";

/// Clear network actions — avoids confusing share vs scan icons.
Future<void> showNetworkConnectSheet(
  BuildContext context, {
  required VoidCallback onShareInvitation,
  required VoidCallback onJoinInvitation,
}) {
  return showModalBottomSheet<void>(
    context: context,
    showDragHandle: true,
    builder: (ctx) {
      return SafeArea(
        child: Padding(
          padding: const EdgeInsets.fromLTRB(8, 0, 8, 16),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              ListTile(
                leading: const Icon(Icons.qr_code_2, size: 32),
                title: const Text("Share my invitation"),
                subtitle: const Text("QR code and link for others to reach you"),
                onTap: () {
                  Navigator.pop(ctx);
                  onShareInvitation();
                },
              ),
              const Divider(height: 1),
              ListTile(
                leading: const Icon(Icons.qr_code_scanner, size: 32),
                title: const Text("Join someone"),
                subtitle: const Text("Scan their QR or paste their invitation link"),
                onTap: () {
                  Navigator.pop(ctx);
                  onJoinInvitation();
                },
              ),
            ],
          ),
        ),
      );
    },
  );
}
