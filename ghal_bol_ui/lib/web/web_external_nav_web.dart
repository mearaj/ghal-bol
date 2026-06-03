// ignore_for_file: avoid_web_libraries_in_flutter, deprecated_member_use

import "dart:html" as html;

import "package:ghal_bol_ui/web/web_site_links.dart";

void openExternalUri(String uri, {bool newTab = false}) {
  if (newTab) {
    html.window.open(uri, "_blank");
    return;
  }
  html.window.location.assign(uri);
}

void openSitePath(String path) {
  html.window.location.assign(path);
}

/// Starts the Linux bundle download without navigating away or blanking the page.
void downloadLinuxBundle() {
  final href = Uri.base.replace(path: WebSiteLinks.linuxArtifactPath).toString();

  // Hidden iframe: browser downloads in the background; Flutter UI stays mounted.
  final iframe = html.IFrameElement()
    ..style.display = "none"
    ..src = href;
  html.document.body?.append(iframe);

  // Fallback for browsers that ignore iframe downloads on same origin.
  final anchor = html.AnchorElement(href: href)
    ..download = "ghal-bol-linux-x64.tar.gz"
    ..style.display = "none";
  html.document.body?.append(anchor);
  anchor.click();

  Future<void>.delayed(const Duration(seconds: 30), () {
    iframe.remove();
    anchor.remove();
  });
}
