// ignore_for_file: avoid_web_libraries_in_flutter

import "package:ghal_bol_ui/web/web_browser_context_web.dart";

/// Href for [WebInviteOpenButton]. Use `ghalbol://` — works in Chrome; not in WebViews.
String inviteOpenButtonHref({
  required String httpsInvite,
  required String appUri,
}) {
  final ctx = readWebInviteBrowserContext();
  if (ctx.isEmbeddedInAppBrowser) {
    // In-app browsers cannot launch custom schemes; avoid intent:// (same failure).
    return httpsInvite;
  }
  return appUri;
}
