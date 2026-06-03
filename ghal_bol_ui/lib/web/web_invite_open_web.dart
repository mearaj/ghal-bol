// ignore_for_file: avoid_web_libraries_in_flutter, deprecated_member_use

import "dart:html" as html;

import "package:ghal_bol_ui/invite_uri_codec.dart";

/// Chrome blocks [html.window.location.assign] for custom schemes unless the
/// navigation comes from a real `<a>` click. Prefer [WebInviteOpenButton] (Link widget).
void _openViaAnchor(String href) {
  final anchor = html.AnchorElement(href: href)
    ..style.display = "none"
    ..rel = "noopener";
  html.document.body?.append(anchor);
  anchor.click();
  anchor.remove();
}

bool _isAndroidBrowser() {
  final ua = html.window.navigator.userAgent.toLowerCase();
  return ua.contains("android");
}

/// Intent URL without `package=` so Play (`com.ghalbol`) and dev (`com.ghalbol.debug`) both match.
String? androidIntentUriForInvite({required String appUri}) {
  final uri = Uri.tryParse(appUri);
  if (uri == null || uri.scheme != kGhalBolConnectAppScheme) return null;
  final pathPart = "${uri.host}${uri.path}";
  final query = uri.hasQuery ? "?${uri.query}" : "";
  return "intent://$pathPart$query"
      "#Intent;scheme=$kGhalBolConnectAppScheme;action=android.intent.action.VIEW;end";
}

void openInviteInApp({required String httpsInvite, required String appUri}) {
  if (_isAndroidBrowser()) {
    final intent = androidIntentUriForInvite(appUri: appUri);
    if (intent != null) {
      _openViaAnchor(intent);
      return;
    }
  }
  _openViaAnchor(appUri);
}
