# Android — opening `https://ghalbol.com/connect/…` in the app

The Flutter app registers invite URLs in `AndroidManifest.xml` and handles them via `app_links` (`invite_deep_link.dart`).

## 1. App build (done in repo)

- `https://ghalbol.com/connect/<public_key_hex>` (`android:autoVerify="true"`)
- `https://www.ghalbol.com/connect/<public_key_hex>` (`android:autoVerify="true"`)
- `ghalbol://connect/<public_key_hex>`

After unlock, the hub runs the same join flow as paste/scan.

## 2. Host verification (required for HTTPS without browser)

For links tapped in Chrome to **open Ghal Bol directly** (not only “Open with…”), publish Digital Asset Links on the website:

**URL:** `https://ghalbol.com/.well-known/assetlinks.json` (and `www.ghalbol.com` — same file via Hosting)

**In repo:** `ghal_bol_ui/web/.well-known/assetlinks.json` — copied into `build/web` on `flutter build web`. Deploy with [WEB_SITE.md](WEB_SITE.md). SHA-256 source: `web/.well-known/README.txt`.

Example (replace SHA-256 cert fingerprints if template):

```json
[
  {
    "relation": ["delegate_permission/common.handle_all_urls"],
    "target": {
      "namespace": "android_app",
      "package_name": "com.ghalbol",
      "sha256_cert_fingerprints": [
        "PLAY_OR_UPLOAD_KEY_SHA256_HERE"
      ]
    }
  }
]
```

Play App Signing: use the **app signing key** fingerprint from Play Console → App integrity, not only your upload key.

Dev installs (`com.ghalbol.debug` / `com.ghalbol.release`) need **their own** entries with debug/release keystore fingerprints, or test `ghalbol://connect/…` links (custom scheme, no `assetlinks`).

## 3. Verify on device

```bash
adb shell pm get-app-links com.ghalbol
adb shell am start -a android.intent.action.VIEW -d "https://ghalbol.com/connect/YOUR_66_HEX_PK"
```

If verification fails, Android keeps opening the browser until `assetlinks.json` is correct.

**Web invite page (research-backed):**

| Where the link opens | Open in Ghal Bol | Reliable fallback |
|----------------------|------------------|-------------------|
| **Chrome / Samsung Internet** (address bar browser) | `ghalbol://` link works when Play app installed | Copy app link → paste in Ghal Bol |
| **WhatsApp / Instagram / in-app WebView** | **Does not work** — WebView cannot launch `ghalbol://` or `intent://` (toast: “App not installed…”) | ⋮ → **Open in Chrome**, or copy app link → paste in app |

This is **not** “friend didn’t install Ghal Bol.” Same APK from Play; different **browser shell**. See `web_browser_context_web.dart` and the banner on the invite page when UA is embedded.

**Verified HTTPS App Links:** If `pm get-app-links com.ghalbol` shows `ghalbol.com: verified`, tapping `https://ghalbol.com/connect/…` in **Chrome** should open the app directly (no web page). If the web page appears, the link was opened in an in-app browser or App Links are not verified on that device.

**Dev builds:** `flutter run` installs `com.ghalbol.debug`. Play App Links target `com.ghalbol`; use the web button (`ghalbol://`) or a debug `assetlinks` entry for sideload testing.
