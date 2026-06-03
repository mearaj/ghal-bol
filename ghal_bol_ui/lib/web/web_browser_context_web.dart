// ignore_for_file: avoid_web_libraries_in_flutter, deprecated_member_use

import "dart:html" as html;

/// How the invite page is loaded in the mobile browser (Chrome vs in-app WebView).
class WebInviteBrowserContext {
  const WebInviteBrowserContext({
    required this.isEmbeddedInAppBrowser,
    required this.isAndroidChrome,
    required this.userAgent,
  });

  /// WhatsApp / Instagram / Facebook / generic Android WebView — cannot launch apps.
  final bool isEmbeddedInAppBrowser;

  /// Standalone Chrome on Android — can open `ghalbol://` from a user tap.
  final bool isAndroidChrome;

  final String userAgent;
}

WebInviteBrowserContext readWebInviteBrowserContext() {
  final ua = html.window.navigator.userAgent;
  final lower = ua.toLowerCase();
  final embedded = _isEmbeddedInAppBrowser(lower);
  final androidChrome = !embedded &&
      lower.contains("android") &&
      lower.contains("chrome") &&
      !lower.contains("edg/");
  return WebInviteBrowserContext(
    isEmbeddedInAppBrowser: embedded,
    isAndroidChrome: androidChrome,
    userAgent: ua,
  );
}

bool _isEmbeddedInAppBrowser(String lower) {
  const embeddedMarkers = [
    "whatsapp",
    "instagram",
    "fb_iab",
    "fbav",
    "fban",
    "twitter",
    "line/",
    "telegram",
    "snapchat",
  ];
  for (final m in embeddedMarkers) {
    if (lower.contains(m)) return true;
  }
  // Android System WebView (not Chrome Custom Tabs): …; wv) …
  if (lower.contains("android") && lower.contains(" wv)")) return true;
  if (lower.contains("android") && lower.contains("; wv)")) return true;
  return false;
}
