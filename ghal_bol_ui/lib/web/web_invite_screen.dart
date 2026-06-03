import "package:flutter/material.dart";
import "package:flutter/services.dart";

import "package:ghal_bol_ui/identity_display_name.dart";
import "package:ghal_bol_ui/invite_uri_codec.dart";
import "package:ghal_bol_ui/web/web_browser_context.dart";
import "package:ghal_bol_ui/web/web_external_nav.dart";
import "package:ghal_bol_ui/web/web_header_graphic.dart";
import "package:ghal_bol_ui/web/web_invite_open_button.dart";
import "package:ghal_bol_ui/web/web_page_shell.dart";
import "package:ghal_bol_ui/web/web_site_links.dart";

class WebInviteScreen extends StatelessWidget {
  const WebInviteScreen({super.key});

  static const routeName = "/invite";

  Future<void> _copy(BuildContext context, String text, String label) async {
    await Clipboard.setData(ClipboardData(text: text));
    if (!context.mounted) return;
    ScaffoldMessenger.of(context).showSnackBar(
      SnackBar(content: Text("$label copied")),
    );
  }

  @override
  Widget build(BuildContext context) {
    final wire = connectInviteWireFromUri(Uri.base);
    final theme = Theme.of(context);
    final colorScheme = theme.colorScheme;
    final browser = readWebInviteBrowserContext();

    if (wire == null) {
      return WebPageShell(
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            const WebHeaderGraphic(),
            const SizedBox(height: 20),
            Text(
              "Invalid invitation link",
              style: theme.textTheme.titleLarge,
            ),
            const SizedBox(height: 12),
            Text(
              "Use a link like https://ghalbol.com/connect/<public_key_hex>",
              style: theme.textTheme.bodyMedium?.copyWith(
                color: colorScheme.onSurfaceVariant,
              ),
            ),
            const SizedBox(height: 24),
            TextButton(
              onPressed: () => openExternalUri(Uri.base.origin),
              child: const Text("Back to home"),
            ),
          ],
        ),
      );
    }

    final pk = wire["public_key_hex"]?.toString() ?? "";
    final alias = wire["peer_alias"]?.toString();
    final https = inviteHttpsStringFromUri(Uri.base)!;
    final appUri = inviteAppUriFromHttps(https)!;
    final display = ghalBolIdName(publicKeyHex: pk, customAlias: alias);
    final embedded = browser.isEmbeddedInAppBrowser;

    return WebPageShell(
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          const WebHeaderGraphic(),
          const SizedBox(height: 20),
          Text(
            "Join on Ghal Bol",
            style: theme.textTheme.headlineSmall?.copyWith(fontWeight: FontWeight.w600),
          ),
          const SizedBox(height: 8),
          Text(
            display,
            style: theme.textTheme.titleMedium?.copyWith(color: colorScheme.primary),
          ),
          if (embedded) ...[
            const SizedBox(height: 16),
            Material(
              color: colorScheme.errorContainer.withValues(alpha: 0.35),
              borderRadius: BorderRadius.circular(12),
              child: Padding(
                padding: const EdgeInsets.all(14),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      "Opened inside another app (e.g. WhatsApp)",
                      style: theme.textTheme.titleSmall?.copyWith(
                        fontWeight: FontWeight.w600,
                        color: colorScheme.onErrorContainer,
                      ),
                    ),
                    const SizedBox(height: 8),
                    Text(
                      "This built-in browser cannot open Ghal Bol — that is why you may see "
                      "\"App not installed\" even when the app is on Play Store.\n\n"
                      "Do one of the following:\n"
                      "• Tap ⋮ (menu) → Open in Chrome or Samsung Internet, then use Open in Ghal Bol.\n"
                      "• Or copy the app link below → open Ghal Bol → Join → paste link.",
                      style: theme.textTheme.bodyMedium?.copyWith(
                        color: colorScheme.onErrorContainer,
                        height: 1.45,
                      ),
                    ),
                  ],
                ),
              ),
            ),
          ] else ...[
            const SizedBox(height: 16),
            Text(
              "Tap Open in Ghal Bol below. If nothing happens, copy the app link and paste it "
              "inside Ghal Bol (Join / paste invite).",
              style: theme.textTheme.bodyMedium?.copyWith(
                color: colorScheme.onSurfaceVariant,
                height: 1.4,
              ),
            ),
          ],
          const SizedBox(height: 20),
          if (!embedded)
            WebInviteOpenButton(
              httpsInvite: https,
              appUri: appUri,
              label: "Open in Ghal Bol",
            ),
          if (embedded) const SizedBox(height: 4),
          FilledButton.icon(
            onPressed: () => _copy(context, https, "Web link"),
            icon: const Icon(Icons.copy),
            label: const Text("Copy web link"),
          ),
          const SizedBox(height: 8),
          FilledButton.icon(
            onPressed: () => _copy(context, appUri, "App link"),
            icon: const Icon(Icons.copy),
            label: const Text("Copy app link"),
          ),
          const SizedBox(height: 28),
          const Divider(),
          const SizedBox(height: 16),
          Text("Install the app", style: theme.textTheme.titleSmall),
          const SizedBox(height: 12),
          FilledButton.icon(
            onPressed: () => openExternalUri(WebSiteLinks.playStore),
            icon: const Icon(Icons.android),
            label: const Text("Google Play"),
          ),
          const SizedBox(height: 10),
          OutlinedButton.icon(
            onPressed: () => openSitePath(WebSiteLinks.linuxDownloadPagePath),
            icon: const Icon(Icons.computer),
            label: const Text("Linux download"),
          ),
          const SizedBox(height: 24),
          TextButton(
            onPressed: () => openExternalUri(Uri.base.origin),
            child: const Text("Home"),
          ),
        ],
      ),
    );
  }
}
