import "package:flutter/material.dart";

import "package:ghal_bol_ui/web/web_external_nav.dart";
import "package:ghal_bol_ui/web/web_header_graphic.dart";
import "package:ghal_bol_ui/web/web_page_shell.dart";
import "package:ghal_bol_ui/web/web_site_links.dart";

class WebHomeScreen extends StatelessWidget {
  const WebHomeScreen({super.key});

  static const routeName = "/";

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final colorScheme = theme.colorScheme;

    return WebPageShell(
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          const WebHeaderGraphic(),
          const SizedBox(height: 24),
          Text(
            "Ghal Bol",
            textAlign: TextAlign.center,
            style: theme.textTheme.headlineMedium?.copyWith(
              fontWeight: FontWeight.w700,
            ),
          ),
          const SizedBox(height: 12),
          Text(
            "Peer-to-peer encrypted chat. No phone number. Direct connection when peers are online.",
            textAlign: TextAlign.center,
            style: theme.textTheme.bodyLarge?.copyWith(
              color: colorScheme.onSurfaceVariant,
              height: 1.45,
            ),
          ),
          const SizedBox(height: 36),
          FilledButton.icon(
            onPressed: () => openExternalUri(WebSiteLinks.playStore),
            icon: const Icon(Icons.android),
            label: const Text("Get it on Google Play"),
            style: FilledButton.styleFrom(
              padding: const EdgeInsets.symmetric(vertical: 14),
            ),
          ),
          const SizedBox(height: 12),
          OutlinedButton.icon(
            onPressed: () => openSitePath(WebSiteLinks.linuxDownloadPagePath),
            icon: const Icon(Icons.computer),
            label: const Text("Download for Linux"),
            style: OutlinedButton.styleFrom(
              padding: const EdgeInsets.symmetric(vertical: 14),
            ),
          ),
          const SizedBox(height: 32),
          Text(
            "Invitation links open in the app when it is installed.",
            textAlign: TextAlign.center,
            style: theme.textTheme.bodySmall?.copyWith(
              color: colorScheme.onSurfaceVariant,
            ),
          ),
        ],
      ),
    );
  }
}
