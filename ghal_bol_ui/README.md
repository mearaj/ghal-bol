# ghal_bol_ui — Flutter shell

Thin UI over **`ghal_bol`** (Rust): unlock, hub, chat, QR invites, calls, delivery ticks from native transcript state. Product logic stays in Rust — see [AGENTS.md](../AGENTS.md) and [docs/DESIGN.md](../docs/DESIGN.md).

| Target | Entry | P2P |
|--------|--------|-----|
| Android | `bootstrap_native.dart` | `:p2p` process + JNI |
| Linux desktop | `bootstrap_native.dart` | `ghal_bol_daemon` in `linux/native/libexec/` |
| Web (marketing only) | `bootstrap_web.dart` | Not compiled — static site in `lib/web/` |

**Coord / env:** [env/README.md](env/README.md)

**Public website (ghalbol.com):** [docs/WEB_SITE.md](../docs/WEB_SITE.md)

**Native rebuild:** `../scripts/sync_ghal_bol_native_for_flutter.sh` (Linux), `../scripts/pack_android_workspace_jni_libs.sh` (Android)
