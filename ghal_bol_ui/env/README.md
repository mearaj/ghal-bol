# Environment (`env/.env.*`)

Coordination server URL and related flags for **all platforms**, including **Android APKs**.

## Resolution order (coord URL)

1. **`--dart-define=GHAL_BOL_COORD_URL=…`** (compile-time; also `--dart-define-from-file=env/.env.development`)
2. **OS environment** — `export GHAL_BOL_COORD_URL=…` before `flutter run` (desktop; adb shell rarely used)
3. **Bundled `env/.env.*`** — copied into the app at build time (`pubspec.yaml` lists `env/`)
4. **Platform default** — desktop: `http://127.0.0.1:8765`; Android emulator: `http://10.0.2.2:8765` when `GHAL_BOL_ANDROID_EMULATOR=true`
5. **Native preferences** — last URL applied via `GhalBolCoord.setBaseUrl` (Rust only)

Invite QR / links are **public key only** (optional `?alias=`). No `?coord=` in URIs.

## Android / iOS setup

```bash
cd ghal_bol_ui
cp env/.env.development.example env/.env.development
# edit GHAL_BOL_COORD_URL (LAN IP for a physical phone, not 127.0.0.1)
flutter pub get
flutter run -d android
```

`env/.env.development` is bundled into the APK because `pubspec.yaml` includes the whole `env/` folder. Rebuild after changing the file.

Alternative without editing the file:

```bash
flutter run --dart-define=GHAL_BOL_COORD_URL=http://192.168.1.10:8765
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
| `GHAL_BOL_COORD_URL` | Coordination server base URL (no trailing slash) |
| `GHAL_BOL_COORD_INSECURE_TLS` | `true` / `1` for self-signed HTTPS |
| `GHAL_BOL_ANDROID_EMULATOR` | `true` when using Android emulator (default coord `10.0.2.2:8765`) |

Storage paths are owned by **`ghal_bol`** (Rust). Flutter does not choose data directories.
