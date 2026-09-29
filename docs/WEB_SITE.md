# Ghal Bol web (static site)

Public site at **https://ghalbol.com** and **https://www.ghalbol.com** (Firebase Hosting). Marketing home page, **Linux desktop download**, and **invite handoff** for `/connect/…` links. Chat runs in the desktop app (`ghal_bol_app`) or the Android app, not in the browser.

## Architecture

| Piece | Location |
|-------|----------|
| Pages | `web/index.html` — one document, routes from `location.pathname` |
| Hosting config | Repo root `firebase.json` → `web/` |
| Android App Links | `web/.well-known/assetlinks.json` |
| Linux bundle | `web/downloads/ghal-bol-linux-x64.tar.gz` when you place it there before deploy |

Firebase serves existing files first (the tarball, `assetlinks.json`). The rewrite to `index.html` applies when no static file matches.

## Routes

| URL | Page |
|-----|------|
| `/` | Home — Play Store + **Download for Linux** |
| `/download/linux` | Linux instructions + link to the tarball |
| `/downloads/ghal-bol-linux-x64.tar.gz` | Static file (when present in `web/downloads/`) |
| `/connect/<public-key-hex>` | Invite handoff — `ghalbol://connect/…`, copy web link, copy app link |
| `/.well-known/assetlinks.json` | Digital Asset Links for verified HTTPS → app |

Optional query on invites: `?alias=Name` only. The public key is 64 or 66 hex characters.

## Preview

```bash
cd web && python -m http.server 8080
```

| Page | Example URL |
|------|-------------|
| Home | `http://localhost:8080/` |
| Linux download | `http://localhost:8080/download/linux` |
| Invite | `http://localhost:8080/connect/<hex>?alias=Name` |

## Linux desktop bundle

Build `ghal_bol_app` in release on **x86_64 Linux**, pack the binary with the libraries it needs, and save the archive as `web/downloads/ghal-bol-linux-x64.tar.gz` before `firebase deploy`.

## Firebase Hosting

### One-time setup

1. [Firebase console](https://console.firebase.google.com/) — project + **Hosting**.
2. Custom domains **ghalbol.com** and **www.ghalbol.com**.
3. Locally:

```bash
npm install -g firebase-tools
firebase login
cp .firebaserc.example .firebaserc
```

### Deploy

```bash
firebase deploy --only hosting
```

### Pre-deploy checklist

| Item | Action |
|------|--------|
| `assetlinks.json` | Play app-signing SHA-256 in `web/.well-known/assetlinks.json` |
| Linux tarball | `web/downloads/ghal-bol-linux-x64.tar.gz` present if the download button should work |
| Verify live | `/`, `/download/linux`, the tarball URL, `/.well-known/assetlinks.json` |

## Invite link behaviour

| Situation | What happens |
|-----------|----------------|
| **Android + app installed + App Links verified** | `https://ghalbol.com/connect/…` opens **Ghal Bol** directly |
| **Android + app installed, verification pending** | Browser shows this page; **Open in Ghal Bol** is a real `<a href="ghalbol://connect/…">` |
| **No app installed** | Web page links to Play Store or the Linux download |
| **Desktop browser** | Same invite page. If Ghal Bol is installed, `ghalbol://connect/…` opens the app and joins after unlock. Otherwise paste the link in **Join**. |

**Open in Ghal Bol** is an HTML link. Do not navigate to `ghalbol://` from script alone; Chrome blocks that unless the user taps the link.

Details: [ANDROID_APP_LINKS.md](ANDROID_APP_LINKS.md), [GHAL_BOL_URI_SCHEME.md](GHAL_BOL_URI_SCHEME.md).

Play listing: `https://play.google.com/store/apps/details?id=com.ghalbol`.
