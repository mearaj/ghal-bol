# Environment (`env/.env.*`)

Coordination server URLs and related flags for **all platforms**, including **Android APKs**.

## Resolution order (coord URLs)

1. **Bundled `env/.env.*`** — `env/.env.development` (debug) or `env/.env.production` (release), copied at build time (`pubspec.yaml` lists `env/`)
2. **`--dart-define=GHAL_BOL_COORD_URLS=…`** (compile-time override)
3. **OS environment** — `export GHAL_BOL_COORD_URLS=…` before `flutter run`
4. **Native preferences** — last URLs applied via `GhalBolCoord.setBaseUrls` (Rust only)

There are **no** hardcoded coord URLs in the app — configure `env/.env.development` and `env/.env.production`.

Invite QR / links are **public key only** (optional `?alias=`). No `?coord=` in URIs.

## Android / iOS setup

```bash
cd ghal_bol_ui
cp env/.env.development.example env/.env.development
# edit GHAL_BOL_COORD_URLS (LAN IP for a physical phone, not 127.0.0.1)
flutter pub get
flutter run -d android
```

`env/.env.development` is bundled into the APK because `pubspec.yaml` includes the whole `env/` folder. Rebuild after changing the file.

Alternative without editing the file:

```bash
flutter run --dart-define=GHAL_BOL_COORD_URLS='["http://192.168.1.10:8765"]'
```

Or:

```bash
flutter run --dart-define-from-file=env/.env.development
```

## Desktop setup

Same `env/.env.development` file; debug builds also read it from disk if you run from the repo.

Pick the device explicitly — `flutter run` does **not** auto-select Linux:

```bash
flutter devices
flutter run -d linux
flutter run -d android
```

## Production coord + release builds

Production server: **`https://coord.ghalbol.com`** (set in `env/.env.production`).

| Goal | Command |
|------|---------|
| Linux release test | `./scripts/sync_ghal_bol_native_for_flutter.sh` then `cd ghal_bol_ui && flutter run --release -d linux` |
| Linux release bundle | `flutter build linux --release` |
| Android release APK | `./scripts/pack_android_workspace_jni_libs.sh` then `cd ghal_bol_ui && flutter build apk --release` |
| Debug against prod coord | `flutter run -d linux --dart-define-from-file=env/.env.production` |

Smoke the live server from repo root:

```bash
COORD_URL=https://coord.ghalbol.com ./ghal_bol_server/deploy/smoke_coord.sh
```

## Keys

| Key | Purpose |
|-----|---------|
| `GHAL_BOL_COORD_URLS` | Coord server list — JSON array or comma-separated (no trailing slashes) |
| `GHAL_BOL_COORD_INSECURE_TLS` | `true` / `1` for self-signed HTTPS |

Storage paths are owned by **`ghal_bol`** (Rust). Flutter passes `app_namespace` only:

| Platform / build | Namespace | Linux path (example) |
|------------------|-----------|----------------------|
| `flutter run -d linux` | `com.ghalbol.debug` | `~/.local/share/com.ghalbol.debug/` |
| `flutter run --release -d linux` / `flutter build linux` | `com.ghalbol` | `~/.local/share/com.ghalbol/` |
| Android `flutter run` | `com.ghalbol.debug` | `app_flutter/com.ghalbol.debug/` (keystore); `app_flutter/ghal_bol/` (stores) |
| Android release / Play | `com.ghalbol` | `app_flutter/` (keystore); `app_flutter/ghal_bol/` (stores) |
