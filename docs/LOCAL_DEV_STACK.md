# Local dev stack — coordination server + Ghal Bol apps

Use one **`ghal_bol_server`** instance on your desktop for both **Linux desktop** and **Android** Flutter builds. P2P chat uses **libp2p** between peers (QUIC/TCP + Noise); the server stores **public keys → dial endpoints** (Tier 1).

---

## 1. Start the server (desktop)

**Default** (desktop + phone on LAN — listens on `0.0.0.0:8765`):

```bash
./ghal_bol_server/deploy/run_server.sh
```

**Ctrl+C:** The script skips `cargo build` when the release binary is up to date, then `exec`s the server so one interrupt stops it. During a rebuild, Ctrl+C may need a second press if a `rustc` child is still finishing. You should see `shutdown signal received` in the log before the shell returns.

**Loopback only** (no LAN clients):

```bash
GHAL_BOL_SERVER_LISTEN=127.0.0.1:8765 ./ghal_bol_server/deploy/run_server.sh
```

(`run_server_lan.sh` is equivalent to the default.)

From the phone’s browser or Termux: `curl http://<desktop-lan-ip>:8765/health` must succeed before the app can look up peers.

Android needs HTTP to your LAN IP; the app enables cleartext for local dev. Rebuild/reinstall the APK after manifest changes.

Smoke test:

```bash
COORD_URL=http://127.0.0.1:8765 ./ghal_bol_server/deploy/smoke_coord.sh
```

Find your LAN IP (example `192.168.1.42`) and verify from another host:

```bash
curl -s http://192.168.1.42:8765/health
```

---

## 2. Rebuild native + run Flutter

**Required after any `ghal_bol` change.** Quit the app first, then from workspace root:

**Linux desktop:**

```bash
./scripts/sync_ghal_bol_native_for_flutter.sh
cd ghal_bol_ui && flutter run
```

**Android phone** (do not use `sync` — it only stages Linux/macOS/Windows artifacts):

```bash
./scripts/pack_android_workspace_jni_libs.sh
cd ghal_bol_ui && flutter run
```

`pack` runs `cargo-ndk` on the host only; it does not call `adb`.

On unlock the app:

1. Unlocks the **`:p2p` / daemon** process (same password as the UI).
2. Sets the coordination URL and passes it into every `p2p_start`.
3. Starts the poll bridge + Android foreground service.
4. On **listening**, registers LAN endpoints with the coord server and re-lookups peer bootstrap addrs.

In **More → App log**, check for `coord registered`, `sync_contacts: node running`, and `bootstrap=N` (N > 0 after the peer is online on the server). For calls, filter **Calls** or search `[Call]` / `ice_`.

---

## 3. Two-device chat test

1. Server: `./ghal_bol_server/deploy/run_server.sh`
2. Desktop: `cd ghal_bol_ui && flutter run` — unlock, show QR
3. Phone: `flutter run` — unlock, **scan** the desktop QR (normal connect flow)
4. Send a message both ways

Desktop and phone on the **same Wi‑Fi** usually need nothing else (mDNS). If they are on **different subnets**, the host QR carries the coord URL automatically when the desktop is listening — the phone picks it up on scan. No extra flags or settings.

---

## 4. Coord URL (automatic — do not configure by hand)

| Client | Default |
|--------|---------|
| Linux desktop | `http://127.0.0.1:8765` |
| Android emulator | `http://10.0.2.2:8765` when built with emulator define |
| Android phone | Set on **scan host QR** (saved for next unlock) |

`--dart-define=GHAL_BOL_COORD_URL=...` is only for odd CI/build experiments, not the normal workflow.

If lookup returns no endpoints, wait until both sides show **listening** in logs (register runs after the DM node publishes a non-loopback TCP listen address).

---

## Related

- [COORDINATION_SERVER.md](COORDINATION_SERVER.md) — API and deploy
- [DESIGN.md](DESIGN.md) — acks and P2P ownership in Rust
- [AGENTS.md](../AGENTS.md) — do not re-implement ack policy in Dart
