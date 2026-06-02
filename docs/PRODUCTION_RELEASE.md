# Production release — P0 / P1 / P2

Step-by-step checklist to ship Ghal Bol. Complete in order.

**Live coord:** `https://coord.ghalbol.com`

---

## P0 — Ship a release build

### P0.1 Release keystore (one-time)

On your dev machine:

```bash
keytool -genkey -v \
  -keystore ~/ghalbol-release.jks \
  -keyalg RSA -keysize 2048 -validity 10000 \
  -alias ghalbol
```

Copy the template and fill in absolute paths and passwords:

```bash
cp ghal_bol_ui/android/key.properties.example ghal_bol_ui/android/key.properties
# edit storeFile, storePassword, keyPassword
```

`key.properties`, `*.jks`, and `*.keystore` are gitignored.

### P0.2 Build release APK (or AAB for Play Store)

```bash
./scripts/pack_android_workspace_jni_libs.sh
cd ghal_bol_ui
flutter build apk --release
# Play Store upload:
# flutter build appbundle --release
```

Outputs:

| Artifact | Path |
|----------|------|
| APK | `ghal_bol_ui/build/app/outputs/flutter-apk/app-release.apk` |
| AAB | `ghal_bol_ui/build/app/outputs/bundle/release/app-release.aab` |

Release builds bundle `env/.env.production` → `GHAL_BOL_COORD_URL=https://coord.ghalbol.com`.

Without `key.properties`, Gradle signs with the debug key (fine for local install only).

### P0.3 Install on test phones

```bash
adb install -r ghal_bol_ui/build/app/outputs/flutter-apk/app-release.apk
```

Or sideload the APK file directly.

### P0.4 Two-device ship test (required)

Use **two physical phones on mobile data** (Wi‑Fi off or different carriers — not the same LAN).

| # | Step | Pass? |
|---|------|-------|
| 1 | Phone A: unlock / create identity | |
| 2 | Phone A: Hub → share invite QR | |
| 3 | Phone B: scan QR (mobile data) | |
| 4 | Phone B: contact appears on A after connect | |
| 5 | A sends text → B receives | |
| 6 | B: single grey tick on A (delivered / `ack_received`) | |
| 7 | B opens room → A gets blue tick (`ack_read`) | |
| 8 | B sends reply → A receives + ticks | |
| 9 | Optional: voice call connects both ways | |
| 10 | Kill app on B, reopen — transcript persists | |

If lookup fails: confirm coord smoke passes from laptop:

```bash
COORD_URL=https://coord.ghalbol.com ./ghal_bol_server/deploy/smoke_coord.sh
```

Debug on device: `:p2p` process logs via `adb logcat | rg ghal_bol`.

**P0 is done when:** signed release APK/AAB builds and the ship test table passes.

---

## P1 — Infra and repo

### P1.1 CI on GitHub

Push `main` with [`.github/workflows/ci.yml`](../.github/workflows/ci.yml). Confirm Actions green:

- Rust unit + P2P integration tests
- `dart analyze --fatal-warnings` + `flutter test`

### P1.2 Coord server survives reboot (GCP VM)

On the VM (`coord.ghalbol.com`):

```bash
# 1. Stop foreground server (Ctrl+C) if running

# 2. Install system unit (edit User + paths if needed)
sudo cp ghal_bol_server/deploy/ghal-bol-server.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable ghal-bol-server
sudo systemctl start ghal-bol-server

# 3. Verify
sudo systemctl status ghal-bol-server
curl -s https://coord.ghalbol.com/health

# 4. Reboot test
sudo reboot
# after reconnect:
curl -s https://coord.ghalbol.com/health
```

See [ghal_bol_server/deploy/README.md](../ghal_bol_server/deploy/README.md).

### P1.3 Commit production changes

From repo root — exclude built binaries (`linux/native/libexec/ghal_bol_daemon` is gitignored).

**P1 is done when:** CI green, coord survives reboot, production branch committed and pushed.

---

## P2 — Play Store

### P2.1 Privacy policy

Publish [PRIVACY_POLICY.md](PRIVACY_POLICY.md) at a public URL (e.g. `https://ghalbol.com/privacy`). Play Console requires this link.

### P2.2 Store listing

Use the draft in [PLAY_STORE_LISTING.md](PLAY_STORE_LISTING.md): short description, full description, category, content rating questionnaire.

### P2.3 Store assets

| Asset | Spec |
|-------|------|
| App icon | 512×512 PNG — `ghal_bol_ui/assets/app_icon.png` as source |
| Feature graphic | 1024×500 PNG |
| Phone screenshots | ≥2, 16:9 or 9:16 |

### P2.4 Play Console upload

1. Create app → `com.ghalbol`
2. Upload `app-release.aab` from P0.2
3. Set privacy policy URL
4. Complete Data safety form (no account; local identity; coord sees presence/endpoints only — see privacy policy)
5. Internal testing track → add testers → roll out

**P2 is done when:** internal testing build is installable from Play Store.

---

## Quick reference

```bash
# Prod coord smoke
COORD_URL=https://coord.ghalbol.com ./ghal_bol_server/deploy/smoke_coord.sh

# Android release
./scripts/pack_android_workspace_jni_libs.sh
cd ghal_bol_ui && flutter build appbundle --release

# Linux release
./scripts/sync_ghal_bol_native_for_flutter.sh
cd ghal_bol_ui && flutter build linux --release
```
