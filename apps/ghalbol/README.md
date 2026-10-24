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
| `resources/fonts/` | Noto Sans global scripts + FA icons + LXGW CJK + Noto Emoji |
| `resources/android/` | Android Manifest template |
| `env/development.env` | Dev coord/delivery URLs, debug app ID |
| `env/production.env` | Prod coord/delivery URLs |
| `build.rs` | Bakes env file into binary at compile time |

## Build & Run

```bash
# Desktop dev (hot reload + Tweaker F12)
cargo ghalbol-dev            # via tools/ghalbol-dev wrapper
makepad run -p ghalbol_app  # direct Makepad command

# Windows dev package
cargo ghalbol-windows-dev    # → dist/windows-dev/

# Android debug APK
cargo ghalbol-android-dev

# Production Linux release
./scripts/package_linux_release.sh

# Production Windows MSI
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

| Build | XDG_DATA_HOME path |
|-------|-------------------|
| Debug | `~/.local/share/com.ghalbol.debug/` |
| Release | `~/.local/share/com.ghalbol/` |

Contains: `keystore_v1.json`, `contacts_v1.json`, `chat_transcript_v1.json`, `preferences_v1.json`
