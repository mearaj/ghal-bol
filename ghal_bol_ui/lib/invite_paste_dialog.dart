import "package:flutter/material.dart";
/// Paste dialog — controller owned by [State] so dispose never races the route pop.
Future<String?> showPasteInviteLinkDialog(BuildContext context) {
  return showDialog<String>(
    context: context,
    builder: (ctx) => const _PasteInviteLinkDialog(),
  );
}

class _PasteInviteLinkDialog extends StatefulWidget {
  const _PasteInviteLinkDialog();

  @override
  State<_PasteInviteLinkDialog> createState() => _PasteInviteLinkDialogState();
}

class _PasteInviteLinkDialogState extends State<_PasteInviteLinkDialog> {
  final _ctrl = TextEditingController();

  @override
  void dispose() {
    _ctrl.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return AlertDialog(
      title: const Text("Paste invitation link"),
      content: TextField(
        controller: _ctrl,
        autofocus: true,
        decoration: const InputDecoration(
          hintText: "https://ghalbol.com/connect/… or ghalbol://connect/…",
        ),
        maxLines: 4,
      ),
      actions: [
        TextButton(onPressed: () => Navigator.pop(context), child: const Text("Cancel")),
        FilledButton(
          onPressed: () => Navigator.pop(context, _ctrl.text.trim()),
          child: const Text("Use link"),
        ),
      ],
    );
  }
}
