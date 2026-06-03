import "dart:async";

import "package:app_links/app_links.dart";
import "package:flutter/foundation.dart" show kIsWeb;

import "ghalbol_connect_invite.dart";

/// Android/iOS invite URLs opened from outside the app (`https://ghalbol.com/connect/…`, `ghalbol://…`).
abstract final class InviteDeepLink {
  InviteDeepLink._();

  static final AppLinks _appLinks = AppLinks();
  static StreamSubscription<Uri>? _subscription;
  static String? _pendingUri;

  /// Hub sets this to run the same join flow as paste/scan.
  static void Function(String uri)? onInviteUri;

  static Future<void> install() async {
    if (kIsWeb) return;
    await _subscription?.cancel();
    _subscription = null;
    try {
      final initial = await _appLinks.getInitialLink();
      _remember(initial);
      _subscription = _appLinks.uriLinkStream.listen(_remember);
    } catch (_) {
      // Plugin unavailable on desktop — ignore.
    }
  }

  static void _remember(Uri? uri) {
    if (uri == null) return;
    final raw = uri.toString();
    if (GhalBolConnectInvite.tryParseInviteUri(raw) == null) return;
    _pendingUri = raw;
    final handler = onInviteUri;
    if (handler != null) {
      handler(raw);
      _pendingUri = null;
    }
  }

  /// Consumed once when [ChatHubScreen] is ready after unlock.
  static String? takePending() {
    final p = _pendingUri;
    _pendingUri = null;
    return p;
  }

  static Future<void> dispose() async {
    await _subscription?.cancel();
    _subscription = null;
    onInviteUri = null;
  }
}
