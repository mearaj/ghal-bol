import "package:flutter/material.dart";

import "package:ghal_bol_ui/web/web_external_nav.dart";
import "package:ghal_bol_ui/web/web_header_graphic.dart";
import "package:ghal_bol_ui/web/web_page_shell.dart";
import "package:ghal_bol_ui/web/web_site_links.dart";

/// `/download/linux` — same hero as home; download starts in the background.
class WebLinuxDownloadScreen extends StatelessWidget {
  const WebLinuxDownloadScreen({super.key});

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
            "Ghal Bol for Linux",
            textAlign: TextAlign.center,
            style: theme.textTheme.headlineMedium?.copyWith(
              fontWeight: FontWeight.w700,
            ),
          ),
          const SizedBox(height: 12),
          Text(
            "Extract the archive and run the app from the folder inside.",
            textAlign: TextAlign.center,
            style: theme.textTheme.bodyLarge?.copyWith(
              color: colorScheme.onSurfaceVariant,
              height: 1.45,
            ),
          ),
          const SizedBox(height: 28),
          FilledButton.icon(
            onPressed: () {
              downloadLinuxBundle();
              ScaffoldMessenger.of(context).showSnackBar(
                const SnackBar(
                  content: Text("Download started — check your browser downloads."),
                  duration: Duration(seconds: 4),
                ),
              );
            },
            icon: const Icon(Icons.download),
            label: const Text("Download Linux bundle"),
            style: FilledButton.styleFrom(
              padding: const EdgeInsets.symmetric(vertical: 14),
            ),
          ),
          const SizedBox(height: 12),
          OutlinedButton(
            onPressed: () => openSitePath(WebSiteLinks.homePath),
            child: const Text("Back to home"),
          ),
        ],
      ),
    );
  }
}
