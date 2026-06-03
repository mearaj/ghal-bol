import "package:flutter/material.dart";
import "package:url_launcher/link.dart";

import "package:ghal_bol_ui/web/web_invite_open_target.dart";

/// Real HTML anchor — required on Android Chrome (see Flutter #78524).
class WebInviteOpenButton extends StatelessWidget {
  const WebInviteOpenButton({
    super.key,
    required this.httpsInvite,
    required this.appUri,
    required this.label,
  });

  final String httpsInvite;
  final String appUri;
  final String label;

  @override
  Widget build(BuildContext context) {
    final href = inviteOpenButtonHref(
      httpsInvite: httpsInvite,
      appUri: appUri,
    );
    return Link(
      uri: Uri.parse(href),
      builder: (context, followLink) => FilledButton.icon(
        onPressed: followLink,
        icon: const Icon(Icons.open_in_new),
        label: Text(label),
        style: FilledButton.styleFrom(padding: const EdgeInsets.symmetric(vertical: 14)),
      ),
    );
  }
}
