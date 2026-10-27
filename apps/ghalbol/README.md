# Ghal Bol Chat — Product App

This directory contains the **Ghal Bol** E2EE messenger app, built on Makepad 2.

## Overview

- **Package name:** `ghalbol_app` (Makepad crate name)
- **App ID (prod):** `com.ghalbol`  
- **App ID (dev):** `com.ghalbol.debug`
- **Product brand:** GhalBol — family of communication products
- **Shared core:** `ghalbol_core/` (identity, P2P, delivery, crypto)

## Directory Layout

| Path | Role |
|------|------|
| `src/main.rs` | Makepad 2 UI shell (~5200 lines). Hub, chat, invites, calls |
| `src/i18n.rs` | Multi-language support (25+ languages) |
| `src/phrases.rs` | Phrase table for all supported languages |
| `resources/fonts/` | Noto Sans scripts + Font Awesome (CJK LXGW and Noto Emoji come from `makepad_widgets`) |
| `resources/android/` | Android manifest template, launcher icons, launch theme |
| `env/development.env` | Dev coord/delivery URLs, debug app ID |
| `env/production.env` | Prod coord/delivery URLs |
| `build.rs` | Bakes the selected env file into the binary (`GHALBOL_ENV`, else Cargo profile) |

## Build & Run

```bash
# Desktop dev (hot reload + Tweaker F12)
cargo ghalbol-dev            # via tools/ghalbol-dev wrapper
cargo run -p ghalbol_app     # plain debug run (development env)

# Windows dev package
cargo ghalbol-windows-dev    # → dist/windows-dev/

# Android debug APK
cargo ghalbol-android-dev

# Production Linux release
./scripts/package_linux_release.sh

# Production Windows package (Setup.bat per-user install)
./scripts/package-windows.sh
```

## Environment Variables (from env/*.env)

| Variable | Development | Production |
|----------|-------------|------------|
| `GHALBOL_ENV` | `development` | `production` |
| `GHALBOL_APP_ID` | `com.ghalbol.debug` | `com.ghalbol` |
| `GHALBOL_APP_LABEL` | `Ghal Bol Dev` | `Ghal Bol` |
| `GHALBOL_COORD_URLS` | `[\"https://coord1.ghalbol.com:8443\"]` | `[\"https://coord.ghalbol.com\"]` |
| `GHALBOL_DELIVERY_URL` | `wss://delivery.ghalbol.com:55003` | `wss://delivery.ghalbol.com:55003` |

## Docs

- [`docs/GHALBOL_BRAND.md`](../../docs/GHALBOL_BRAND.md) — Brand identity & naming
- [`docs/DESIGN.md`](../../docs/DESIGN.md) — System architecture design
- [`docs/MAKEPAD_UI.md`](../../docs/MAKEPAD_UI.md) — UI shell details

## Data Paths

The data directory follows `GHALBOL_APP_ID` (`com.ghalbol` or `com.ghalbol.debug`).

| Platform | Release (`com.ghalbol`) | Development (`com.ghalbol.debug`) |
|----------|-------------------------|-----------------------------------|
| Linux | `~/.local/share/com.ghalbol/` | `~/.local/share/com.ghalbol.debug/` |
| Windows | `%LOCALAPPDATA%\com.ghalbol\data\` | `%LOCALAPPDATA%\com.ghalbol\data\com.ghalbol.debug\` |
| Android | app files dir | app files dir `/com.ghalbol.debug/` |

The data directory holds `keystore_v1.json` and `preferences_v1.json`; contacts, the transcript,
voice notes and attachments live in its `ghalbol/` subfolder. Builds from before the
`ghal_bol` → `ghalbol` rename used `ghal_bol/`; the app moves it on first start.
