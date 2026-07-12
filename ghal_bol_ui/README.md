# ghal_bol_ui — Flutter shell

Thin UI over **`ghal_bol`** (Rust): unlock, hub, chat, QR invites, calls, delivery ticks from native transcript state. Product logic stays in Rust — see [AGENTS.md](../AGENTS.md) and [docs/DESIGN.md](../docs/DESIGN.md).

**Session signals:** use **`GhalBolUiSession`** (`lib/ghal_bol_ui_session.dart`) only — `setVisible` + `setRoom` → native `p2p_sync_ui_session`. Do not call deprecated `GhalBolP2p.setForegroundPeer` / `setAppAckReadEnabled` from product code.

**Hub transcript thread id:** pass `hubThreadKey: _selectedConversationKey` into `ChatScreen` — not `activeContact` alone (roster row flickers null on reload and caused cross-room history loss; see [DESIGN.md § Hub chat — stable thread id](../docs/DESIGN.md#hub-chat--stable-thread-id-hubthreadkey--regression-guard)).

**Identity tab — Show private key:** no `SelectableText` / `SelectionArea` on `_identityBody`; dialog `TextEditingController`s owned by `StatefulWidget` dialog `State` (see `invite_paste_dialog.dart`); `dismissTextSelectionForDialog` before chains — [DESIGN.md § Identity tab regression guard](../docs/DESIGN.md#identity-tab--show-private-key-dialog-stack--regression-guard). Android trace: `ghal_bol_ui/flutter_android.log`.

| Target | Entry | P2P |
|--------|--------|-----|
| Android | `bootstrap_native.dart` | `:p2p` process + JNI |

**Android screen off:** after unlock, `ChatHubScreen` runs `AndroidBackgroundReadiness` (notifications, battery optimization, unused-app pause, OEM autostart) **before** P2P starts — see [DESIGN.md § Fixed 2026-07-05](../docs/DESIGN.md#fixed-2026-07-05--android-background-readiness-screen-off).
| Linux desktop | `bootstrap_native.dart` | `ghal_bol_core_daemon` in `linux/native/libexec/` |
| Web (marketing only) | `bootstrap_web.dart` | Not compiled — static site in `lib/web/` |

**Coord / env:** [env/README.md](env/README.md)

**Public website (ghalbol.com):** [docs/WEB_SITE.md](../docs/WEB_SITE.md)

**Native rebuild:** `../scripts/sync_ghal_bol_native_for_flutter.sh` (Linux), `../scripts/pack_android_workspace_jni_libs.sh` (Android)
