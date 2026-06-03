import "package:flutter/material.dart";

import "package:ghal_bol_ui/invite_uri_codec.dart";
import "package:ghal_bol_ui/web/web_home_screen.dart";
import "package:ghal_bol_ui/web/web_invite_screen.dart";
import "package:ghal_bol_ui/web/web_linux_download_screen.dart";
import "package:ghal_bol_ui/web/web_site_links.dart";

/// Static marketing site + `/connect/…` → `ghalbol://` handoff (no P2P on web yet).
class GhalBolWebApp extends StatelessWidget {
  const GhalBolWebApp({super.key});

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: "Ghal Bol",
      debugShowCheckedModeBanner: false,
      theme: ThemeData(
        colorScheme: ColorScheme.fromSeed(seedColor: Colors.deepPurple),
        useMaterial3: true,
      ),
      // With [usePathUrlStrategy], the browser path is not a Flutter route name.
      // Pick the page from [Uri.base] instead of [initialRoute] + [routes].
      home: const _WebRootPage(),
    );
  }
}

class _WebRootPage extends StatelessWidget {
  const _WebRootPage();

  @override
  Widget build(BuildContext context) {
    final path = Uri.base.path;
    if (_isLinuxDownloadPath(path)) {
      return const WebLinuxDownloadScreen();
    }
    if (connectInviteWireFromUri(Uri.base) != null) {
      return const WebInviteScreen();
    }
    return const WebHomeScreen();
  }
}

bool _isLinuxDownloadPath(String path) {
  const p = WebSiteLinks.linuxDownloadPagePath;
  return path == p || path.startsWith("$p/");
}
