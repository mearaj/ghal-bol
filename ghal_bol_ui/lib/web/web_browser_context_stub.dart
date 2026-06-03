/// Non-web builds — invite page is not compiled into native targets.
class WebInviteBrowserContext {
  const WebInviteBrowserContext();

  bool get isEmbeddedInAppBrowser => false;
  bool get isAndroidChrome => false;
}

WebInviteBrowserContext readWebInviteBrowserContext() =>
    const WebInviteBrowserContext();
