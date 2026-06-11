# Environment (`env/.env.*`)

Coordination server URLs for **all platforms**, including **Android APKs**.

## Files

| File | In git | When used |
|------|--------|-----------|
| `env/.env.development` | **Yes** (tracked) | `flutter run` (debug) |
| `env/.env.production` | **No** (gitignored) | `flutter build --release` |

The app loads **only** these two paths — not `*.example`. Both are listed in `pubspec.yaml` and bundled at build time. Create `env/.env.production` locally before a release build (see Production below).

## Resolution order (coord URLs)

1. **Bundled `env/.env.development` or `env/.env.production`** (from `pubspec.yaml`)
2. **`--dart-define=GHAL_BOL_COORD_URLS=…`** (compile-time override)
3. **OS environment** — `export GHAL_BOL_COORD_URLS=…` before `flutter run`
4. **Native preferences** — only when (1–3) did not set URLs

Edit `GHAL_BOL_COORD_URLS` in `env/.env.development` for debug, then **rebuild** the app (hot reload does not rebundle assets).

## Android / iOS

```bash
cd ghal_bol_ui
# edit env/.env.development (default: https://coord.ghalbol.com)
flutter pub get
flutter run -d android
```

## Desktop

```bash
flutter devices
flutter run -d linux
```

## Production

`env/.env.production` is **gitignored** — create it on your machine (never commit secrets or machine-specific URLs):

```bash
cd ghal_bol_ui
printf 'GHAL_BOL_COORD_URLS=https://coord.ghalbol.com\n' > env/.env.production
./scripts/pack_android_workspace_jni_libs.sh
flutter build apk --release
```

## Keys

| Key | Purpose |
|-----|---------|
| `GHAL_BOL_COORD_URLS` | Coord server list — JSON array or comma-separated (no trailing slashes) |
| `GHAL_BOL_COORD_INSECURE_TLS` | `true` / `1` for self-signed HTTPS |
