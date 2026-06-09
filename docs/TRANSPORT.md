# Transport — libp2p data plane

**Status:** **libp2p is the production P2P transport.** A prior plan to replace libp2p with a custom native QUIC/TCP stack was **evaluated and discarded** (May 2026). This document is the canonical reference for how peers connect today.

**For AI / new sessions:** Read [AGENTS.md](../AGENTS.md), [STORY.md](STORY.md), and [DESIGN.md](DESIGN.md) first. Transport changes must **not** move ack policy, outbox, or transcript merge into Flutter. **Connectivity and discovery policy:** [STORY.md](STORY.md) overrides any conflicting guidance here.

---

## Summary

Ghal Bol separates **chat protocol** from **transport**:

| Layer | Implementation |
|-------|----------------|
| **Chat protocol** | `ghal_bol_msg_v1` — signed JSON envelopes on framed streams ([GHAL_BOL_DM_MSG_V1.md](GHAL_BOL_DM_MSG_V1.md)) |
| **Transport** | **libp2p 0.56** swarm in `ghal_bol/src/p2p/chat_server.rs` |
| **Discovery (Tier 1)** | `ghal_bol_server` register/lookup + co-located relay (WAN) + libp2p mDNS (LAN per-peer) |
| **Policy** | Outbox, ack send/retry, foreground gates, transcripts — same module tree regardless of transport |

libp2p is **not** the chat protocol. It provides encrypted connections, stream multiplexing, and dial/listen. All product semantics live in Rust (`chat_server.rs`, `dm_event_handler.rs`, stores).

---

## libp2p stack (current)

Enabled in `ghal_bol/Cargo.toml` and wired in `chat_server.rs`:

| libp2p piece | Role in Ghal Bol |
|--------------|------------------|
| **QUIC / TCP + Noise + Yamux** | Encrypted connections and multiplexing |
| **`libp2p-stream` `/ghal-bol/msg/1.0.0`** | Framed DM channel for `ghal_bol_msg_v1` |
| **mDNS** | LAN discovery of configured peers |
| **Relay + DCUtR** | NAT traversal — reserve a circuit on a **Ghal Bol relay** (co-located with a configured coord server), then DCUtR upgrades to a direct link. **WAN peer discovery does not use Kademlia or public libp2p bootstrap peers** — see [STORY.md](STORY.md) |
| **AutoNAT, UPnP, Identify** | Reachability and peer metadata |
| **Ping** | **Connection keepalive** — periodic pings keep an idle DM/relay link active so it is not dropped by `idle_connection_timeout`; also detects a dead route faster (`PING_INTERVAL_SECS` 15s < idle timeout) |
| **Gossipsub** | **Not used** — do not reintroduce for 1:1 DM |

Identity: one **secp256k1** keypair per device → libp2p **PeerId** via `libp2p-identity` (`keystore_v1.rs`, `peer_id_util.rs`).

---

## Architecture

```text
┌─────────────────────────────────────────────────────────────────┐
│  ghal_bol_ui — screens, hub foreground, poll (unchanged role)   │
└────────────────────────────┬────────────────────────────────────┘
                             │ FFI / daemon JSON-RPC
┌────────────────────────────▼────────────────────────────────────┐
│  ghal_bol — policy + stores (unchanged role)                       │
│  msg_v1, outbox, pending_read_acks, dm_event_handler, stores     │
└────────────────────────────┬────────────────────────────────────┘
                             │
┌────────────────────────────▼────────────────────────────────────┐
│  p2p/chat_server.rs — libp2p swarm + sessions                    │
│  Stream protocol /ghal-bol/msg/1.0.0; writer task per peer       │
│  Outbox, ack_received/ack_read, foreground/leave drain         │
└────────────────────────────┬────────────────────────────────────┘
                             │ register / heartbeat / lookup
┌────────────────────────────▼────────────────────────────────────┐
│  ghal_bol_server — Tier-1 coordination (no message bodies)       │
└─────────────────────────────────────────────────────────────────┘
```

**Process split (Linux / Android):** libp2p runs **out-of-process** in `ghal_bol_daemon` or Android `:p2p` (`GhalBolP2pService`). The UI process loads `libghal_bol.so` for identity and store I/O; both share the same data directory.

---

## Connectivity lifecycle (authoritative)

This section encodes the **override rules at the end of [AGENTS.md](../AGENTS.md)** — they take precedence over any other guidance here. The whole transport must behave this way; if any other doc disagrees, it is wrong and must be updated to match this.

1. **Start on first unlock, run in the background.** After the user unlocks the first time, the node (`ghal_bol_daemon` / Android `:p2p`) starts and **keeps running** regardless of UI state. UI lock/suspend never stops the node, poll, or ack loops.
2. **Watch the network continuously.** A 1s interface/profile poll plus OS connectivity callbacks (`notify_network_change`) detect internet up/down and Wi‑Fi ↔ mobile ↔ LAN handovers. Loss or change triggers recovery without user-visible disruption (`handle_network_path_change`, `run_wan_recovery_pass`).
3. **Find both addresses fast.** The node determines its **LAN** address (interface scan / mDNS) and its **globally reachable** address (public listen, AutoNAT/UPnP, or a relay `/p2p-circuit` when behind NAT/CGNAT).
4. **Register at coord as soon as a reachable address exists, and keep it fresh.** Once a publishable global endpoint (public TCP or relay circuit) is known, register with `ghal_bol_server` and re-register on the heartbeat tick and on every endpoint change (`coord_runtime.rs`). Registration waits only until at least one publishable listen addr exists.
5. **WAN must always work when both peers have internet and coord is reachable.** This is the baseline guarantee — coord lookup + relay reservation + DCUtR, never gated off by being on Wi‑Fi/LAN.
6. **LAN is per-peer and additive.** Use the LAN path **only** for a contact actually discovered on the local LAN (mDNS), never globally. If the LAN is lost, that peer transparently falls back to the normal WAN path with no user-visible change.
7. **Coord down ≠ app offline, but WAN discovery pauses.** When coord is unreachable, **do not** fall back to Kademlia DHT or public libp2p bootstrap peers for WAN peer discovery ([STORY.md](STORY.md)). **LAN (mDNS) still works** for contacts on the local network. Keep retrying **all configured coord servers** at a regular interval — never stop. **WAN requires coord + relay** when both peers have internet.
8. **Internet/coord recovery is immediate** — when internet or coord comes back, the continuous watch detects it within seconds and resumes WAN registration/lookup across the coord list without a libp2p restart.

The ultimate goal is **strong, reliable, smooth** peer interaction. coord + libp2p are sufficient for this across WAN and LAN; do not regress these guarantees for performance or simplicity.

---

## Discovery (Tier 1)

Typical WAN flow ([GHAL_BOL_URI_SCHEME.md](GHAL_BOL_URI_SCHEME.md), [COORDINATION_SERVER.md](COORDINATION_SERVER.md)):

1. Guest scans host QR → stores `public_key_hex`.
2. Both peers register endpoints with `ghal_bol_server`.
3. Lookup `GET /v1/peers/{public_key_hex}` → dial returned endpoints via libp2p.
4. Open `/ghal-bol/msg/1.0.0` stream; speak `ghal_bol_msg_v1`.

**WAN first:** coord register/lookup on **every configured coord server** + relay circuit (and public TCP when registered). **LAN only** when mDNS shows the configured peer on the same network (direct TCP). No Kademlia or public-bootstrap peer discovery when coord paths fail — keep retrying coord.

Coord publishes `tcp`, `quic`, and `libp2p` multiaddrs; `coord_runtime.rs` and `dm_transport/addr.rs` help filter and rank dial targets before libp2p dials.

### LAN vs WAN dial policy

**WAN first** for every configured contact. **LAN** (mDNS → direct TCP) **only** when that peer is discovered on the local LAN — not from coord RFC1918 addrs alone.

- **WAN:** coord lookup + relay circuit (on CGNAT/mobile-data) + public TCP when registered. Lookup tries configured coord servers in order; **stop on first success** for that dial attempt; on reconnect after a drop, repeat the full list.
- **LAN exception:** mDNS `Discovered` for the contact → prefer direct TCP for that peer.
- **Mobile-data:** no blind peer-id dials when coord is configured — explicit relay multiaddrs only.

#### Immediate LAN shift + fast WAN fallback (mDNS-driven)

LAN is faster and stronger, so a contact discovered on the LAN must shift onto it **immediately**, and a contact that **leaves** the LAN must fall back to WAN **without** a long stall — both are explicit STORY requirements. `dial_mdns_peer` / the mDNS `Expired` handler in `chat_server.rs` enforce this:

- **`Mdns::Discovered`** → `note_peer_on_local_lan` + dial. If we are **already connected but only over a relay circuit** (`!peer_has_direct_connection`), `dial_lan_upgrade` dials the direct LAN multiaddr with `PeerCondition::NotDialing` (so the relay link does not block it), throttled per peer (`LAN_UPGRADE_DIAL_THROTTLE_MS`, 10s). This is **additive** — the existing connection is never torn down, so WAN keeps working if the LAN dial fails; new DM/media streams ride the faster direct link once it is up. Direct vs relay is tracked per connection in `ConnectionEstablished`/`ConnectionClosed` via `is_relay_circuit_multiaddr(endpoint.get_remote_address())` (`peers_direct_conns`).
- **`Mdns::Expired`** → `forget_peer_on_local_lan` drops the per-peer LAN preference immediately (instead of waiting out `PEER_LAN_SEEN_TTL_MS` = 180s), so dial ranking returns to **WAN-first** at once, and `kick_dm_peer_discovery` re-runs coord/relay lookup so the peer stays reachable over the internet without delay. If the (now LAN-less) connection later drops, urgent reconnect (§ "Steady connection") takes over.

Do **not** tear down the existing connection on LAN discovery (that would drop in-flight messages); the upgrade is additive and the stream follows the better path on reopen.

See [DESIGN.md](DESIGN.md) § “Dial strategy — WAN first”. Do not add Dart dial policy or RFC1918 /24 guessing from coord (regression: long connect stalls).

### Roaming

- **This device** — Android connectivity in `:p2p`, 1s interface profile poll, WAN relay recovery when coord URL is set; optional `p2p_notify_network_change` from UI resume.
- **Coord tick** — periodic lookup (~5s) plus immediate lookup when send is queued and peer is not connected.

### Steady connection when both peers are online (do not regress)

The link between two online contacts must stay **steady** — no idle drops, and fast recovery from a transient blip — so messages are not delayed by a full reconnect. Three mechanisms in `chat_server.rs` enforce this:

1. **Keepalive ping** — `ChatBehaviour.ping` pings every `PING_INTERVAL_SECS` (15s), comfortably under `SWARM_IDLE_CONNECTION_TIMEOUT_SECS` (45s Android / 300s desktop). A healthy-but-quiet chat connection is therefore never dropped between messages. Do **not** remove ping or raise the interval above the idle timeout.
2. **Urgent reconnect** — on `dm connection closed`, the peer’s key enters a bounded urgent window (`DM_RECONNECT_URGENT_WINDOW_MS`, 30s) via `mark_dm_reconnect_urgent`. While urgent (`is_pk_reconnect_urgent`), coord lookup **skips** the `peer_not_on_server` 404 backoff and the 1s upkeep tick retries reconnect immediately, instead of waiting for the 5s coord tick or the exponential backoff. The window is cleared on successful reconnect.
3. **Reserve on all configured coord relays in parallel, throttled per relay** — `try_relay_reservations` issues `listen_on(/p2p-circuit)` to every connected **Ghal Bol relay** (from the configured coord server list) that is not already circuit-listening, and `try_relay_reservation` enforces a per-relay throttle (`RELAY_RESERVE_THROTTLE_MS`). Do **not** use public IPFS bootstrap peers for relay reservation or peer discovery. The anti-pattern is re-issuing `listen_on` **every tick** (a 1s storm), **not** covering all relays once: serializing onto a single relay let one pending-but-never-accepted reservation block the others, so WAN readiness took minutes or never came up. Per-relay throttling keeps the parallel fan-out storm-free.

---

## Multiple coord / relay servers

The app accepts a **list** of coord server base URLs (today a single entry: `https://coord.ghalbol.com`; the API is an array for future redundancy). Each entry is a full **coord + relay** pair — HTTP presence plus a co-located Circuit Relay v2 node (`GET /v1/relay` on that host).

| Action | Policy |
|--------|--------|
| **Register** | Register presence (and relay circuit addr) on **every** reachable coord in the list |
| **Lookup** | When dialling a peer, try coord servers in order; **stop on first successful lookup + connect** |
| **Reconnect** | After a connection drop while internet is active, repeat lookup across the full list |
| **Coord unreachable** | Keep retrying all entries on the regular interval; **LAN (mDNS) unaffected** |

Do not substitute Kademlia DHT or public libp2p bootstrap peers when a coord lookup fails — WAN discovery requires coord/relay ([STORY.md](STORY.md)).

---

## Ghal Bol relay (co-located with coord)

When neither peer is directly reachable (home‑NAT desktop ⇄ CGNAT phone), WAN needs a **Ghal Bol relay** that reliably grants Circuit Relay v2 reservations. `ghal_bol_server` runs its **own** relay node next to each HTTP coordinator. The HTTP API stays a lightweight presence phone book; the relay only carries brief NAT‑traversal traffic until **DCUtR** upgrades the client pair to a direct connection. **Public IPFS bootstrap peers are not used** for peer discovery or relay reservation.

**Server (`ghal_bol_server/src/relay.rs`)**
- Circuit Relay v2 + Identify (`/ghal-bol/1.0.0`) + Ping over **TCP + Noise + Yamux** (libp2p 0.56, protocol‑identical to the client).
- Stable ed25519 identity persisted at `<data_dir>/relay_ed25519.key` → constant PeerId across restarts. (The relay's **own** node key is ed25519 — that is fine; it is infrastructure, not a user identity.)
- **The relay's `libp2p` MUST enable the `secp256k1` feature** (`ghal_bol_server/Cargo.toml`). Ghal Bol **clients authenticate with their secp256k1 device identity** (golden rule 7 / [IDENTITY.md](IDENTITY.md)). The Noise handshake authenticates the remote's identity public key, so a relay built **without** `secp256k1` cannot decode/verify a secp256k1 client and **drops the connection mid‑handshake** — the client sees `Decode(Io(UnexpectedEof))`, the circuit listener closes (`addrs=[]`), `coord_registered=false`, and **no real device can ever reserve a circuit** (every device uses a secp256k1 key). A minimal probe using an ed25519 key will *appear* to work and hide this — always test the relay with a **secp256k1** key (`PROBE_SECP256K1=1` in `examples/relay_probe.rs`).
- Env: `GHAL_BOL_RELAY_ENABLE` (default on), `GHAL_BOL_RELAY_LISTEN` (default `0.0.0.0:4002`), `GHAL_BOL_RELAY_PUBLIC_HOST` (→ advertises `/dns4/<host>/tcp/<port>`) or `GHAL_BOL_RELAY_PUBLIC_ADDRS` (comma‑separated multiaddrs). **The relay TCP port must be open to the internet**; advertise the public host or clients cannot reserve.
- `GET /v1/relay` → `{ enabled, peer_id, addrs }` (addrs are dialable bases without `/p2p/<id>`).

**Client (`ghal_bol`)**
- At swarm startup, for **each** configured coord URL, `coord_runtime::fetch_all_ghalbol_relays` fetches `/v1/relay` and resolves dialable bases to `/ip4/<public>/tcp/<port>/p2p/<id>`. The relay is reserved via `try_ghalbol_probe_style_circuit_listen` — a direct `listen_on(/ip4/.../tcp/<port>/p2p/<id>/p2p-circuit)` that mirrors the verified `examples/relay_probe.rs` path — and its `/p2p-circuit` is registered in coord presence on that server, then retried by the recovery loop in § "Steady connection".
- **Cached to disk** (`<data_dir>/ghalbol_relay.json` per coord host): a successful fetch is persisted; if the startup fetch is **unreachable** (flaky mobile data at launch, HTTP/TLS front hiccup) the client reuses the last cached relay for that host. A reachable-but-**disabled** relay clears the cache (honors the operator turning it off). The PeerId is stable, so the cache stays valid across restarts.
- **Network handovers** (wifi ⇄ mobile ⇄ different LAN): relay re-reservation rides `handle_network_path_change` → `retry_stalled_relay_reservations`, so the circuit is re-reserved and re-registered on the new path without a libp2p restart.

---

## Helper modules (not a separate transport)

| Path | Role |
|------|------|
| `ghal_bol/src/p2p/chat_server.rs` | libp2p swarm, streams, outbox, ack policy |
| `ghal_bol/src/p2p/network_transport.rs` | Network profile, relay resolution, dial/publish addr helpers (no Kademlia) |
| `ghal_bol/src/p2p_runtime.rs` | Background node thread, poll queue |
| `ghal_bol/src/dm_transport/` | **Dial-address helpers only** — parse coord endpoints; libp2p still uses `Multiaddr` on the wire |
| `ghal_bol/src/coord_runtime.rs` | Register/listen snapshot, lookup → dial addrs |

There is **no** standalone native TCP/QUIC listener stack. Do not assume `dm_transport/` replaces libp2p.

---

## Footprint (approximate, May 2026)

| Metric | Value |
|--------|-------|
| Dependency crates (`cargo tree -p ghal_bol`) | ~967 |
| libp2p-named crates | ~70 |
| Release `libghal_bol.so` / `ghal_bol_daemon` | ~20 MB unstripped, ~15 MB stripped |
| `chat_server.rs` | ~4k lines — transport + session + ack policy |

Accepting libp2p keeps NAT traversal, mDNS, and relay tooling at the cost of binary size and build time. The discarded native-stack plan targeted smaller binaries but was not pursued.

---

## Stable FFI / daemon surface

Do not rename without a version bump:

- `p2p_start` / `p2p_stop` (avoid stop on contact upsert — [AGENTS.md](../AGENTS.md))
- `register_dm_peer` / `sync_contacts`
- `send_text_dm`
- `p2p_poll` / `apply_p2p_event_json`
- `p2p_set_foreground_peer` / `p2p_set_app_ack_read_enabled`
- Coord: register, heartbeat, lookup ([COORDINATION_SERVER.md](COORDINATION_SERVER.md))

---

## Invariants (do not break when touching transport)

| Invariant | Owner |
|-----------|--------|
| `ghal_bol_msg_v1` envelope, ack kinds, `ref_id` rules | `msg_v1.rs`, [GHAL_BOL_DM_MSG_V1.md](GHAL_BOL_DM_MSG_V1.md) |
| Recipient sends `ack_received` always; `ack_read` only in-room; leave backlog | `chat_server.rs` — not Dart |
| Sender outbox until peer acks | Same |
| Flutter **never** sends acks; poll refreshes UI only | [DESIGN.md](DESIGN.md) |
| Guest scans host QR; host may have zero contacts | `connect_invite_v1`, `dm_event_handler` |
| `p2p_poll` → `apply_p2p_event_json` → disk → UI reload | `p2p_runtime.rs`, `dm_event_handler.rs` |
| No `p2p_stop` on every contact change | `register_dm_peer` / `sync_contacts` |
| Keepalive **ping** keeps idle DM/relay links up (interval < idle timeout) | `chat_server.rs` `chat_behaviour` |
| Reconnect is **urgent** after a DM drop (no 404 backoff, 1s retries) | `mark_dm_reconnect_urgent` / `is_pk_reconnect_urgent` |
| Relay reservations cover **all configured coord relays**, throttled per relay (no 1s storm) | `try_relay_reservations` / `try_relay_reservation` |
| LAN discovery upgrades a relay-only link (additive, never tears down WAN); LAN loss drops the LAN pref immediately + re-kicks WAN | `dial_mdns_peer` / `dial_lan_upgrade` / `forget_peer_on_local_lan` |

---

## AI handoff — common mistakes

1. **Reimplementing ack policy in Dart** — forbidden; see DESIGN.md.
2. **Assuming libp2p was removed** — it was not; read this file.
3. **Reintroducing gossipsub** for 1:1 DM — wrong model.
4. **Requiring mutual QR** — guest-only host key from QR is intentional.
5. **Clearing `pending_read_acks` on leave** — breaks DESIGN leave backlog.
6. **Restarting libp2p on every contact upsert** — use hot `register_dm_peer` instead.
7. **Kademlia / public-bootstrap WAN discovery when coord is down** — forbidden; WAN requires coord/relay ([STORY.md](STORY.md)). LAN (mDNS) still works.
8. **Slow WAN fallback after LAN loss** — mDNS `Expired` must re-kick coord/relay lookup immediately; do not wait on LAN TTL.

---

## Related documents

| Doc | Relationship |
|-----|----------------|
| [DESIGN.md](DESIGN.md) | Canonical product behaviour |
| [GHAL_BOL_DM_MSG_V1.md](GHAL_BOL_DM_MSG_V1.md) | Wire format and ack kinds |
| [GHAL_BOL_URI_SCHEME.md](GHAL_BOL_URI_SCHEME.md) | Invite URLs and QR |
| [COORDINATION_SERVER.md](COORDINATION_SERVER.md) | Run/test coord, local dev stack |
| [PREMIUM_SERVICES.md](PREMIUM_SERVICES.md) | Optional Tier 3 paid relay |

---

## Changelog

| Date | Change |
|------|--------|
| 2026-06-09 | **STORY alignment — coord-required WAN, no KAD/bootstrap discovery:** WAN peer discovery requires configured coord/relay servers; Kademlia DHT and public libp2p bootstrap peers are not fallbacks. LAN (mDNS) still works when coord is down. Added § "Multiple coord / relay servers". See [STORY.md](STORY.md). |
| 2026-06-06 | **Immediate LAN shift + fast WAN fallback (STORY):** mDNS `Discovered` now upgrades a relay-only link to a direct LAN connection (`dial_lan_upgrade`, `PeerCondition::NotDialing`, per-peer throttle; additive, never tears down WAN); per-connection direct/relay tracking added (`peers_direct_conns`). mDNS `Expired` drops the LAN preference immediately (no 180s TTL wait) and re-kicks WAN discovery. See § "Immediate LAN shift + fast WAN fallback". |
| 2026-06-06 | **Relay secp256k1 fix (root cause of WAN failure):** `ghal_bol_server`'s `libp2p` was missing the `secp256k1` feature, so the relay dropped every secp256k1 client (i.e. every real device) mid Noise handshake (`Decode(UnexpectedEof)` → circuit listener `addrs=[]` → `coord_registered=false`). Added `secp256k1` (+`ed25519`) to the server. `examples/relay_probe.rs` gained `PROBE_SECP256K1=1` because the ed25519-only probe masked the bug. Removed debug-only scaffolding committed during the hunt (relay-reservation repro test module, `keycheck` example). |
| 2026-06-06 | **Ghal Bol relay co-located with coord** (`ghal_bol_server/src/relay.rs`, `GET /v1/relay`): clients reserve a circuit on our own reliably-granting relay (preferred over public IPFS bootstraps) and register that `/p2p-circuit` in presence — fixes WAN for NAT⇄CGNAT pairs where neither side is directly reachable. |
| 2026-06-06 | WAN regression fix: relay reservations fan out to **all** eligible bootstraps again (per-relay throttle), reverting the one-at-a-time scheme that serialized behind a stuck pending reservation and stalled WAN for minutes. |
| 2026-06-05 | Steady-connection hardening: keepalive `ping`, urgent DM reconnect (no 404 backoff), one-at-a-time relay reservation. |
| 2026-05-31 | Canonical transport doc; libp2p confirmed as production stack. |
