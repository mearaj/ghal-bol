# Ghal Bol daemon — integrator model (precompiled engine + SDK)

**Status:** Canonical integrator architecture. Wire contract: `ghal_bol_core/src/daemon/client_api.rs`. **SDK:** Rust `ghal_bol_core::daemon::{IntegratorConfig, DaemonClient, …}`; Dart mirror `ghal_bol_ui/lib/daemon_client_api.dart`.

**Goal:** Any app on any UI stack integrates **`ghal_bol_core_daemon`** (precompiled binary / Android `:p2p` native bundle) **without patching Ghal Bol Rust source**. The daemon does **not** know which integrator app is connected — only that the client speaks the JSON-RPC contract and satisfies **integrator obligations** (present UI, session sync, consume wakes).

`ghal_bol_ui` is **reference integrator #1**, not the specification.

---

## Architecture

```text
┌─────────────────────────────────────────────────────────────┐
│ Integrator app (any language / toolkit)                      │
│  • Your UI, navigation, OS glue                              │
│  • SDK client: typed RPC + poll + wake handlers            │
│  • Implements: present window, unlock screen, call UI      │
└───────────────────────────┬─────────────────────────────────┘
                            │ Unix socket JSON-RPC
                            │ + namespace-scoped runtime files
┌───────────────────────────▼─────────────────────────────────┐
│ ghal_bol_core_daemon (precompiled — do not fork to integrate)     │
│  • libp2p, outbox, acks, coord/WAN, network truth          │
│  • Emits poll events + wake markers                        │
│  • Ends calls when last UI socket closes                   │
└─────────────────────────────────────────────────────────────┘
```

| Layer | Owns | Must not own (integrator) |
|-------|------|---------------------------|
| **Daemon** | P2P, outbox, ack send/retry, transcript merge on poll, coord register/lookup/dial, read-gate policy | Layout, widgets, platform permission UX |
| **Integrator** | Screens, composer, tick **display**, window present, notifications tap → navigation | Coord HTTP loops, ack retries, optimistic ticks, dial policy |
| **SDK** (client) | Socket I/O, `DaemonMethod` names, path helpers, poll loop utilities | Product logic |

Wire details: [DESIGN.md § UI integrator contract](DESIGN.md#ui-integrator-contract-daemon-owned). Method enum: [`client_api.rs`](../ghal_bol_core/src/daemon/client_api.rs).

---

## Precompiled daemon + configurable SDK

### What integrators ship

| Platform | Engine artifact | SDK |
|----------|-----------------|-----|
| **Linux desktop** | `ghal_bol_core_daemon` binary (bundle under `libexec/` or PATH) | `ghal_bol_core::daemon` (Rust), [`daemon_client_api.dart`](../ghal_bol_ui/lib/daemon_client_api.dart) (Dart mirror) |
| **Android** | `lib_ghal_bol_core.so` + `:p2p` foreground service (same Rust node) | Same Dart mirror over app-private Unix socket |
| **In-process (dev)** | `lib_ghal_bol_core.so` FFI | Optional; not the multi-process integrator path |

Integrators **configure**, they do **not** edit `ghal_bol_core/src/…`:

- `app_namespace` on every `unlock` / data path (identity isolation)
- Environment variables below (runtime + socket isolation)
- Coord URL via `coord_set_base_url` RPC
- Desktop app id for OS wake (`gtk-launch` / D-Bus) — your `.desktop` `StartupWMClass` / application id

### SDK responsibilities

1. Connect / reconnect to the daemon socket (`RpcConnection`, `DaemonClient`).
2. Typed wrappers for each `DaemonMethod` (constants + `call` / `callState`).
3. Poll helper (`DaemonClient.pollEvent`).
4. Wake helpers (`takeUnlockWake`, `takeIncomingCallWake`).
5. **`IntegratorConfig`**: runtime dir + socket path from `app_namespace` (must match daemon env).
6. Parity check: `./scripts/check_daemon_sdk_parity.sh`.

Integrator obligations (present UI, session sync) remain in the host app — see below.

---

## Multiple integrators on one device

Each integrator is a **separate product identity**. They must not share daemon socket, wake files, or keystore paths.

| Resource | Isolation key | Example |
|----------|---------------|---------|
| Keystore, contacts, transcript | `app_namespace` | `com.myapp.chat` → `~/.local/share/com.myapp.chat/ghal_bol/` |
| Daemon Unix socket | Per-namespace runtime dir | `$XDG_RUNTIME_DIR/ghal_bol/com.myapp.chat/p2p.sock` |
| Wake markers (`unlock_wake`, …) | Same runtime dir | `$XDG_RUNTIME_DIR/ghal_bol/com.myapp.chat/unlock_wake` |
| UI presence | Same runtime dir | `…/ui_present` |
| Daemon process | **One daemon per namespace** | Spawn with matching `GHAL_BOL_APP_NAMESPACE` |

**Rule:** The daemon and SDK client for integrator A must use the **same** `app_namespace` and runtime configuration. Connecting integrator B's UI to integrator A's socket is unsupported and may corrupt session semantics.

### Configuration reference

Environment variables read by **`ghal_bol_core_daemon`** and path helpers (highest priority first):

| Variable | Purpose |
|----------|---------|
| `GHAL_BOL_DAEMON_SOCKET` | Explicit socket path (overrides default). Client must use the same path. |
| `GHAL_BOL_RUNTIME_DIR` | Explicit runtime directory for wake files + default socket parent. |
| `GHAL_BOL_APP_NAMESPACE` | Integrator id; scopes runtime dir to `$XDG_RUNTIME_DIR/ghal_bol/<namespace>/` when unset above. |
| `GHAL_BOL_UNLOCK_GRACE_SECS` | Linux autostart unlock wake delay (default 10). |
| `GHAL_BOL_VERBOSE_LOG` | Forward Rust debug logs (daemon process). |

**Default socket** when `GHAL_BOL_APP_NAMESPACE=com.example.app` is set (required for Linux reference app):

```text
$XDG_RUNTIME_DIR/ghal_bol/com.example.app/p2p.sock
```

Reference app (`ghal_bol_ui`) always sets `GHAL_BOL_APP_NAMESPACE` to [`kGhalBolAppNamespace`](../ghal_bol_ui/lib/ghal_bol_constants.dart) on daemon spawn and autostart. Android uses app-private `filesDir/ghal_bol/p2p.sock` via `:p2p` — not this path.

### Spawn checklist (Linux)

1. Choose unique `app_namespace` (e.g. reverse-DNS bundle id).
2. Start daemon with `GHAL_BOL_APP_NAMESPACE=<namespace>` (and optional `GHAL_BOL_DAEMON_SOCKET`).
3. SDK client connects to the matching socket.
4. On UI startup, write `ui_present` under the same runtime dir; clear on exit.
5. Install autostart `.desktop` with the same `Environment=` lines (see `ghal_bol_ui` autostart as reference).

---

## Integrator obligations

The daemon expects the host app to provide these **behaviours**. Names match the documented `UiIntegratorCallbacks` trait in Rust — implement in your language, not by patching the daemon.

| Signal / state | Integrator must |
|----------------|-----------------|
| `unlock_wake` / notification | Present main window; show password / identity screen; call `unlock` RPC. |
| `incoming_call_wake` | Present window; navigate to call UI; consume wake via RPC. |
| App `resumed` / `inactive` / room open/close | Call `p2p_sync_ui_session` (`ui_visible`, `room_public_key_hex`). See DESIGN.md § UI session contract. |
| User sends message | `p2p_send_text_dm` (daemon owns outbox retry). |
| Poll `dm_message` / `stores_updated` | Refresh display from disk/FFI — **do not** send acks from poll. |
| Process exit / last socket close | Best-effort `ui_process_exiting`; daemon force-ends active call on EOF. |
| Login reconnect | `ui_session_prepare_reconnect` before reconnecting socket. |

**Linux desktop note:** Keystore unlock may still use in-process FFI in the UI process while P2P runs in the daemon. Both must use the same `app_namespace` and data dir. A future SDK release may expose all storage ops via RPC; until then document FFI + daemon split for desktop integrators.

---

## Replacing `ghal_bol_ui`

Any replacement (Qt, Tauri, Swift, etc.) must:

1. Bundle or spawn the **same precompiled** `ghal_bol_core_daemon` / Android `:p2p` artifacts.
2. Use the **contract** (`client_api.rs` + `daemon_client_api.dart`) for all daemon RPCs — no duplicated method strings.
3. Implement **integrator obligations** above.
4. Use a **distinct** `app_namespace` unless intentionally sharing identity with Ghal Bol.

`ghal_bol_ui` remains the reference implementation: [`GhalBolDaemonClient`](../ghal_bol_ui/lib/src/ghal_bol_core_daemon_client_io.dart) (platform spawn + socket RPC), plus [`P2pEventBridge`](../ghal_bol_ui/lib/p2p_event_bridge.dart), [`GhalBolUiSession`](../ghal_bol_ui/lib/ghal_bol_ui_session.dart).

---

## Protocol stability

- RPC method names are **`DaemonMethod::ALL`** (36 methods) — add new methods only in `client_api.rs` + SDK mirror + parity tests.
- Breaking param changes require a future **`protocol_version`** field (not yet on wire; plan before 1.0 SDK).
- Do not rely on undeclared RPCs (e.g. socket `shutdown` is daemon-internal, not part of `DaemonMethod`).

---

## Related docs

| Doc | Content |
|-----|---------|
| [DESIGN.md § UI integrator contract](DESIGN.md#ui-integrator-contract-daemon-owned) | RPC + wake tables |
| [DESIGN.md § UI session contract](DESIGN.md#ui-session-contract-integrator-app--native-p2p) | Read receipts / foreground room |
| [AGENTS.md](../AGENTS.md) | Agent rules — daemon owns logic |
| [TRANSPORT.md](TRANSPORT.md) | libp2p / coord (daemon-internal) |
