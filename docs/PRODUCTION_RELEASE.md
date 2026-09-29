# Production release

Checklist for shipping Ghal Bol. The UI is `ghal_bol_app` (Makepad 2). It links `ghal_bol_core` in-process.

**Live coord:** `https://coord.ghalbol.com`

---

## P0 — Desktop release

### P0.1 Build and pack

On x86_64 Linux, with the Makepad system libraries installed (X11, Wayland, EGL/GL, ALSA):

```bash
./scripts/package_linux_release.sh
```

That writes `web/downloads/ghal-bol-linux-x64.tar.gz`. Extract it and run `./run.sh`.

Debug data directory: `~/.local/share/com.ghalbol.debug/`. Release: `~/.local/share/com.ghalbol/`.

On first launch the app writes a login autostart entry and a desktop menu file for the binary you started. If a keystore already exists and the session is locked, it posts an unlock notification.

### P0.2 Two-device test

Use two machines that are not on the same LAN (or one on mobile data via the Android build below).

| # | Step |
|---|------|
| 1 | A: create identity and save the encrypted backup |
| 2 | A: My invitation, share the link or QR |
| 3 | B: Scan QR, QR picture, or Join → paste |
| 4 | A sees B after connect |
| 5 | A sends text → B receives |
| 6 | B: single tick on A (`ack_received`) |
| 7 | B opens the room → A gets the read tick (`ack_read`) |
| 8 | Voice call connects; video call shows local and remote pictures |
| 9 | Quit and reopen — transcript is still there |

Coord smoke from a laptop:

```bash
COORD_URL=https://coord.ghalbol.com ./ghal_bol_coord/deploy/smoke_coord.sh
```

---

## P0 — Android package

The phone UI is the same `ghal_bol_app` binary, built by cargo-makepad. Package id `com.ghalbol`. The manifest template is `ghal_bol_app/resources/android/AndroidManifest.xml.template` (camera, microphone, network, notifications, `ghalbol://` and `https://ghalbol.com/connect/`).

```bash
./scripts/build_android_app.sh          # APK
./scripts/build_android_app.sh --aab    # Play App Bundle
```

Signing for Play uses cargo-makepad’s `--keystore` on `build-aab` (see `cargo makepad android --help`). A debug signature is only for a local install.

Install:

```bash
adb install -r target/makepad-android-apk/ghal_bol_app/apk/app-release.apk
```

The exact APK path is printed by the build. Repeat the two-device table with one phone on mobile data.

The Makepad Android activity is the process. Messages while the screen is off depend on the OS keeping that process alive. A separate boot receiver is not compiled into cargo-makepad’s Java list, so a reboot still needs the user to open Ghal Bol once.

---

## P1 — Repo and servers

### P1.1 CI

`.github/workflows/ci.yml` runs `cargo test` for core, coord, and delivery, then `cargo check -p ghal_bol_app` and the app language tests.

Local checks for the UI contract:

```bash
cargo test -p ghal_bol_core --lib -- --test-threads=1 identity_invite_password_and_locale outbound_send_marks_an_unknown_contact_known availability_presets_persist_and_cap_at_64 transcript_page_keeps_delivery_and_has_more hidden_room_does_not_send_a_new_read_ack qr_png_roundtrip set_contact_trust_updates_flags
cargo test -p ghal_bol_app --bin ghal_bol_app -- i18n:: bubble_tests
```

Those cover create/unlock, backup reveal, invite join, block, password change, locale, QR decode, contact trust, availability, transcript pages, the hidden-room read gate, delivery ticks, and every language catalog.

### P1.2 Coord survives reboot

See [ghal_bol_coord/deploy/README.md](../ghal_bol_coord/deploy/README.md). After reboot, `curl -s https://coord.ghalbol.com/health` must succeed.

---

## P2 — Site and store

### P2.1 Privacy

`https://ghalbol.com/privacy` is served from `web/index.html`. The long form is [PRIVACY_POLICY.md](PRIVACY_POLICY.md).

### P2.2 Linux download and deploy

```bash
./scripts/package_linux_release.sh
./scripts/deploy_web_firebase.sh
```

Confirm `/`, `/download/linux`, `/privacy`, `/downloads/ghal-bol-linux-x64.tar.gz`, and `/.well-known/assetlinks.json`.

### P2.3 Play listing

Use [PLAY_STORE_LISTING.md](PLAY_STORE_LISTING.md). Icon source: `docs/assets/app_icon.png`. Upload the AAB from P0 Android, set the privacy URL, and roll out internal testing.

Store screenshots are taken from the running app (phone or desktop) and uploaded in Play Console. They are not generated in this repo.
