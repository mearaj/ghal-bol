# Ghal Bol web (static site)

Public site at **https://ghalbol.com** and **https://www.ghalbol.com** (Firebase Hosting). Marketing home page, **Linux desktop download**, and **invite handoff** for `/connect/…` links. Full chat in the browser is **not** implemented — use Android (Play Store) or Linux desktop.

## Architecture

| Piece | Location |
|-------|----------|
| Web-only entry | `ghal_bol_ui/lib/main.dart` → `bootstrap_web.dart` (native app not compiled into web) |
| App shell | `ghal_bol_ui/lib/web/ghal_bol_web_app.dart` — reads **`Uri.base.path`**, not Flutter `initialRoute` / named routes |
| Pages | `lib/web/web_home_screen.dart`, `web_invite_screen.dart`, `web_linux_download_screen.dart` |
| URLs / paths | `lib/web/web_site_links.dart` |
| Invite parsing | `lib/invite_uri_codec.dart` (same rules as Android/desktop) |
| Hosting config | Repo root `firebase.json` → `ghal_bol_ui/build/web` |
| Deploy script | `scripts/deploy_web_firebase.sh` |
| Android App Links | `ghal_bol_ui/web/.well-known/assetlinks.json` → `/.well-known/assetlinks.json` on deploy |
| Linux bundle (static) | `ghal_bol_ui/web/downloads/ghal-bol-linux-x64.tar.gz` → `/downloads/…` on deploy |

Firebase serves **existing files first** (e.g. `.tar.gz`, `assetlinks.json`); the SPA rewrite to `index.html` applies only when no static file matches.

## Routes

| URL | Page |
|-----|------|
| `/` | Home — Play Store + **Download for Linux** |
| `/download/linux` | Linux instructions + link to the tarball |
| `/downloads/ghal-bol-linux-x64.tar.gz` | Static file (when present in `web/downloads/` before web build) |
| `/connect/<66-hex-public-key>` | Invite handoff — `ghalbol://`, **Open in Ghal Bol**, copy link |
| `/.well-known/assetlinks.json` | Digital Asset Links for verified HTTPS → app |

Optional query on invites: `?alias=Name` only.

## Run locally

```bash
cd ghal_bol_ui
flutter run -d chrome
```

| Page | Example URL |
|------|-------------|
| Home | `http://localhost:<port>/` |
| Linux download | `http://localhost:<port>/download/linux` |
| Invite | `http://localhost:<port>/connect/<66-hex-public-key>?alias=Name` |

## Build web output

```bash
cd ghal_bol_ui
flutter build web --release
```

Output: `ghal_bol_ui/build/web/` (includes everything under `ghal_bol_ui/web/`, including `downloads/` and `.well-known/`).

## Linux desktop bundle (for the website)

Build on **x86_64 Linux** so the path is `build/linux/x64/release/bundle/`.

```bash
# From repo root
./scripts/sync_ghal_bol_native_for_flutter.sh
cd ghal_bol_ui
flutter build linux --release
mkdir -p web/downloads
tar -czvf web/downloads/ghal-bol-linux-x64.tar.gz \
  -C build/linux/x64/release \
  bundle
```

**Nicer top-level folder after extract** (optional):

```bash
cd build/linux/x64/release
cp -a bundle ghal-bol-linux-x64
tar -czvf ../../../../web/downloads/ghal-bol-linux-x64.tar.gz ghal-bol-linux-x64
rm -rf ghal-bol-linux-x64
```

Then build and deploy web (tarball must exist **before** `flutter build web`).

Site flow: home **Download for Linux** → `/download/linux` (same hero image as home) → **Download Linux bundle** starts the file in the background without leaving the page.

See also `ghal_bol_ui/web/downloads/README.txt`.

## Firebase Hosting

### One-time setup

1. [Firebase console](https://console.firebase.google.com/) — project + **Hosting**.
2. Custom domains **ghalbol.com** and **www.ghalbol.com** (DNS per Firebase).
3. Locally:

```bash
npm install -g firebase-tools   # or your package manager
firebase login
cp .firebaserc.example .firebaserc
# Edit .firebaserc: set "default" to your Firebase project id
```

`.firebaserc` is local (from `.firebaserc.example`); do not commit real project ids if you prefer.

### Deploy

```bash
./scripts/deploy_web_firebase.sh
```

Or:

```bash
cd ghal_bol_ui && flutter build web --release
cd .. && firebase deploy --only hosting
```

### Pre-deploy checklist

| Item | Action |
|------|--------|
| `assetlinks.json` | Play **app signing** SHA-256 in `web/.well-known/assetlinks.json` (see `web/.well-known/README.txt`) |
| Linux tarball | `web/downloads/ghal-bol-linux-x64.tar.gz` present before web build |
| Verify live | `https://ghalbol.com/`, `/download/linux`, `/downloads/ghal-bol-linux-x64.tar.gz` (file download, not HTML), `/.well-known/assetlinks.json` (JSON) |

Preview locally (optional):

```bash
cd ghal_bol_ui && flutter build web --release
cd .. && firebase emulators:start --only hosting
```

## Invite link behaviour

| Situation | What happens |
|-----------|----------------|
| **Android + app installed + App Links verified** | `https://ghalbol.com/connect/…` opens **Ghal Bol** directly |
| **Android + app installed, verification pending** | Browser → web invite page → `ghalbol://` / intent |
| **No app installed** | Web page → Play Store or Linux download |
| **Desktop browser** | Web invite page; **Open in Ghal Bol** uses a real HTML link (`url_launcher` `Link`) so Chrome allows `ghalbol://`; otherwise copy link / install |

**Android Chrome:** programmatic `window.location` to `ghalbol://` or `intent://` is blocked unless the user taps a native `<a>` — do not use `location.assign` from Flutter `onPressed` alone. The invite page uses `WebInviteOpenButton` (`Link` widget). Dev APKs use `com.ghalbol.debug` — intent URLs must not hard-code `package=com.ghalbol` only.

Details: [ANDROID_APP_LINKS.md](ANDROID_APP_LINKS.md), [GHAL_BOL_URI_SCHEME.md](GHAL_BOL_URI_SCHEME.md), [PEER_DISCOVERY.md](PEER_DISCOVERY.md).

## Configuring public URLs

Edit `ghal_bol_ui/lib/web/web_site_links.dart`:

| Constant | Purpose |
|----------|---------|
| `playStore` | Google Play listing (`com.ghalbol`) |
| `linuxDownloadPagePath` | `/download/linux` (in-app path) |
| `linuxArtifactPath` | `/downloads/ghal-bol-linux-x64.tar.gz` |
| `home` / `homePath` | Canonical site URL and `/` |

Privacy policy for Play Console: host at `https://ghalbol.com/privacy` when ready ([PRIVACY_POLICY.md](PRIVACY_POLICY.md)).
