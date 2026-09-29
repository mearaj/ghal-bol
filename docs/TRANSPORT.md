# Transport — native connect data plane

**Status:** **Text messaging** uses [`ghal_bol_delivery`](GHAL_BOL_DELIVERY.md) (mailbox). **Voice/video calls** use **native connect** (`ghal_bol_core/src/connect/`) — LAN direct when possible, otherwise the coord byte bridge. Native connect is **not** the product path for chat text.

**Scope:** Coord register/heartbeat and mDNS upkeep serve **presence** and **call reachability**. When `GHAL_BOL_DELIVERY_URL` is set, **all** product text uses the delivery mailbox — see `ghal_bol_core/src/text_transport.rs` and [DESIGN.md](DESIGN.md) § Goals.

**For AI / new sessions:** Read [AGENTS.md](../AGENTS.md) and [DESIGN.md](DESIGN.md) first. Transport changes must **not** move ack policy, outbox, or transcript merge into Makepad. **Start here:** § **Connectivity lifecycle** → § **Network truth** → § **Parallel LAN + WAN** → § **LAN ↔ WAN handover**. Transport reachability is **live-only** — see § **Caching policy**.

**Shipping coord (`ghal_bol_coord`):** signed presence (`POST /v1/register`, heartbeat), `GET /v1/peers/{identity}`, and **WAN call bridge** (`/v1/bridge/*` + WSS). See [COORDINATION_SERVER.md](COORDINATION_SERVER.md) and [ghal_bol_coord/README.md](../ghal_bol_coord/README.md).

---

## Summary

Ghal Bol separates **chat protocol** from **transport**:

| Layer | Implementation |
|-------|----------------|
| **Text messaging** | `ghal_bol_delivery` WebSocket mailbox — E2E `delivery_msg_v1` ([GHAL_BOL_DELIVERY.md](GHAL_BOL_DELIVERY.md)) |
| **Calls (LAN + WAN)** | Noise + channel mux on native connect; **WAN** uses coord **byte bridge** ([GHAL_BOL_CONNECT_V1.md](GHAL_BOL_CONNECT_V1.md)) |
| **Call transport** | **native connect** in `ghal_bol_core/src/connect/` (tokio TCP + Noise + mux; mDNS LAN) |
| **Discovery / presence** | `ghal_bol_coord` register/lookup + mDNS (`_ghalbol._tcp.local`) for call dial |
| **Policy** | Text: `delivery_runtime`; calls: `connect/` + `dm_event_handler` |

---

## The prime directive — instant connect at any roster size (canonical)

> Whenever two peers have *any* technically reachable path for a **call** — same LAN (mDNS/direct TCP), coord **public TCP** (when registered), or coord **bridge** — they should **connect within a few seconds**. **Text** uses delivery independently of native connect. Throttles and backoff exist to prevent **storms**, not to delay a peer with **active intent**.

### Scale invariant (non-negotiable)

A user may have **thousands of contacts**, most **stale** (offline, never registered, or coord lookup `404`). **Stale peers must not block a reachable peer.** Coord lookup passes split **urgent** (recent disconnect), **priority** (outbox or foreground chat), and **background** (LRU-capped sweep). Urgent and priority are **uncapped** and run first.

### Instant-connect acceptance criteria (testable)

1. **Reachable ⇒ connected within a few seconds** for a **call** (LAN session or call bridge) — independent of roster size. Text delivery polls separately.
2. **Intent beats backoff** — open chat, queued message, or dropped call session skips 404 backoff for the urgent window.
3. **No assumed-timer stalls** — see § **Event-driven async**.
4. **Bounded coord HTTP** — background lookup cap per tick; no full-roster sequential storms.

### What violates the directive (forbidden)

- Unbounded sequential coord lookups over the full roster.
- Capping **urgent / priority** peers to “reduce coord load.”
- Throttles that delay a peer with active intent because other peers are stale.

---

## Parallel LAN + WAN transport (canonical)

**Policy:** On Wi‑Fi, **LAN discovery** (mDNS + ephemeral TCP listen) and **coord presence** (register/heartbeat when publishable endpoints exist) **run together** so **calls** can dial. They are not mutually exclusive. **Text messaging** always uses `delivery_runtime` when the delivery URL is set — never gated on native connect.

### Both stacks active (product requirement)

| Path | Job while online |
|------|------------------|
| **LAN** (mDNS → direct TCP) | Call sessions when both peers share Wi‑Fi |
| **Coord HTTP** | Presence phone book (public routable TCP when any), heartbeat, bridge signaling for WAN calls |
| **Delivery** (when configured) | **Authoritative text mailbox** — product chat path |

**Required behaviour:**

- **Do not stop coord register/heartbeat** because mDNS found a contact on LAN.
- **Do not stop mDNS browse/listen** because coord registered or a WAN call used the bridge.
- **LAN connect is mDNS event-driven** — not a timer re-dial from cached ports (§ **Ephemeral LAN TCP ports**).
- **Calls:** prefer live LAN writer; if none, `POST /v1/bridge/request` + outbound WSS (`connect/bridge_ws.rs`, `outbound.rs`).
- **Text:** never gated on native connect `chat_ready`; delivery worker owns send/recv.

**Wrong docs/code:** treating text as a LAN/native-connect product path; treating coord as “idle backup” on Wi‑Fi; skipping register while on LAN; UI-owned dial/ack policy.

| Layer | LAN | WAN (calls) | Text |
|-------|-----|-------------|------|
| **Infrastructure** | mDNS browse + ephemeral TCP listen | coord bridge WSS (`/v1/bridge/connect`) | `ghal_bol_delivery` WSS |
| **Per-peer** | Direct TCP session when discovered | Bridge-paired TCP (Noise+mux inside tunnel) | Mailbox only |
| **Dial / discovery** | `Discovered` → dial **that** host:port | Bridge request + pending poll | Delivery register/poll |

**Independence rule:** LAN health must **not** suppress coord heartbeat/register (and vice versa). Delivery ticks are independent of mDNS.

**Handover:** Peer leaves LAN → mDNS `Expired` drops LAN call session; **calls** fall back to bridge when placing/receiving; **text** continues on delivery.

---

## Stream-first connect (wire layer)

One live **channel mux** per contact for native connect (**call signaling** and media substreams). See [DESIGN.md](DESIGN.md).

```text
Per contact (connect worker ~1s upkeep + events):
  if live mux writer for peer:
    drain call signaling on that session
  else if mDNS Discovered:
    dial_peer_tcp(host, port) from event
  else if WAN call needed and no LAN writer:
    bridge_request → WSS connect_bridge_session
  inbound TCP / bridge WSS → accept_inbound_tcp / bridge accept
```

| Principle | Implementation |
|-----------|----------------|
| Symmetric | Inbound accept and outbound dial share the same Noise+mux handler |
| Parallel inputs | mDNS events + coord endpoints (public TCP dial when used) + bridge pending poll |
| One session per peer | `PeerSessionRegistry` — new dial skipped when writer open |

---

## native connect stack (current)

Wired in `ghal_bol_core/src/connect/`:

| Piece | Role |
|-------|------|
| **Tokio TCP + Noise XX + channel mux** | Encrypted call sessions; `/ghal-bol/msg/1.0.0` for call signaling frames |
| **mDNS-SD** (`lan_discovery.rs`, `_ghalbol._tcp.local`) | LAN peer discovery |
| **Coord bridge** (`bridge_client.rs`, `bridge_ws.rs`) | WAN call byte pipe via coord WSS |
| **OS network truth** | `p2p/network_transport.rs`, `android_network.rs`, `linux_network.rs` |

Identity: **secp256k1** device key ([IDENTITY.md](IDENTITY.md)). Product WAN discovery is **coord only** (plus delivery for text); no DHT or public bootstrap peers.

---

## Architecture

```text
┌─────────────────────────────────────────────────────────────────┐
│  ghal_bol_app — screens, hub, poll (display only)                │
└────────────────────────────┬────────────────────────────────────┘
                             │ ghal_bol_core::host
┌────────────────────────────▼────────────────────────────────────┐
│  delivery_runtime — text messaging                                    │
│  connect/ — calls + bridge                                            │
│  coord_runtime — register / heartbeat / lookup                    │
│  dm_event_handler, transcripts, outbox policy                     │
└────────────────────────────┬────────────────────────────────────┘
                             │ HTTPS + WSS
┌────────────────────────────▼────────────────────────────────────┐
│  ghal_bol_coord — presence + bridge (no DM bodies)               │
│  ghal_bol_delivery — text mailbox                                    │
└─────────────────────────────────────────────────────────────────┘
```

**Process:** native connect and delivery run **in the same process** as `ghal_bol_app`. The UI calls `ghal_bol_core::host` in-process.

---

## Connectivity lifecycle (authoritative)

Binding rules — if another doc disagrees, update it to match this section.

1. **Start on unlock, run until log out.** `host::start_network` starts connect + delivery workers. **Lock** leaves the node running. **Log out** stops it.
2. **Watch the network continuously.** `notify_network_change` + `refresh_os_network_truth` on connect upkeep (~1s). See § **Network truth**.
3. **Publish coord endpoints when they change.** Public routable **TCP** listen (from ephemeral port + public IP when available) → `POST /v1/register`. **Re-register** on endpoint change, failed register, handover, or stale presence — **throttle** when unchanged (`should_throttle_register`, heartbeat every ~25s). Details: [COORDINATION_SERVER.md](COORDINATION_SERVER.md).
4. **CGNAT / mobile-only peers** often have **no** public TCP to register. They still **heartbeat** when registered empty/minimal; **WAN calls** use `/v1/bridge/*`; **text** uses delivery only.
5. **LAN is per-peer and additive.** mDNS `Discovered` → dial that contact on LAN. mDNS `Expired` → drop LAN session; do not block delivery or bridge.
6. **Coord unreachable ≠ app offline.** LAN mDNS may still work. Retry **all** `GHAL_BOL_COORD_URLS` on the regular coord tick. Do not invent a second WAN discovery path when coord is down.
7. **Internet/coord recovery is event-driven** — resume register/lookup/bridge poll when HTTP works again; no full-process restart required.
8. **Network switch readiness.** Path change → re-register if publishable endpoints changed, clear coord lookup backoff where applicable, reopen native sessions for **calls**.
9. **Process restart.** On `host::start_network`, re-register when listen endpoints are known; do not trust in-memory presence from a prior process.

**UI vs native:** `ghal_bol_core` owns ack policy, outbox, delivery/read ticks, dial, and coord. Makepad uses **`host::set_app_visible`** and **`host::set_open_room`** only. Poll is **display** — never gate sends or acks in the UI.

---

## Hybrid coord presence (WAN directory)

**Problem:** CGNAT/mobile peers cannot publish a dialable public inbound TCP port. They still need coord for **bridge call signaling** and optional **lookup of peers who do have public TCP**.

**Model (shipping):**

| What | Who | How |
|------|-----|-----|
| **Public routable TCP** (`tcp://host:port`) | **Client** | `POST /v1/register` when the device has a **globally routable** inbound listen (UPnP / port-forward / public IP). Must be the peer’s own socket — **not** RFC1918/CGNAT LAN. |
| **LAN TCP** | **Never on coord** | Same-subnet peers use mDNS only. |
| **WAN call reachability (CGNAT)** | **Bridge** | `POST /v1/bridge/request` + both sides dial `GET /v1/bridge/connect` (WSS). No server-side circuit row. |
| **Text (CGNAT / all)** | **Delivery** | Mailbox — not coord. |

Server rejects non-routable “public” endpoints at `POST` ([`ghal_bol_coord` routes/presence](../ghal_bol_coord/src/routes.rs)). Client: `endpoints_for_coord_register`, `on_listen_dm_addr` in `coord_runtime.rs`.

---

## WAN call bridge (coord)

Opaque byte pipe: coord pairs two **outbound** WebSocket connections; peers run the same Noise + mux as LAN inside the tunnel. Full flow: [GHAL_BOL_CONNECT_V1.md](GHAL_BOL_CONNECT_V1.md) § WAN call bridge; server limits: [COORDINATION_SERVER.md](COORDINATION_SERVER.md) § Bridge limits.

**Client summary (`connect/bridge_client.rs`, `bridge_ws.rs`, `worker.rs`):**

1. Caller: `POST /v1/bridge/challenge` → `POST /v1/bridge/request` (signed).
2. Callee: delivery push and/or `GET /v1/bridge/pending?identity_wire=…` (polled ~3s in connect upkeep).
3. Both dial `connect_url` as **WSS** with `bridge_id` + `token`.
4. Coord forwards binary frames until hangup, idle timeout, or server caps.

**nginx / home coord:** same vhost must proxy **`/v1/bridge/connect`** with WebSocket upgrade headers.

---

## LAN ↔ WAN handover

**Policy:** LAN and coord/delivery are **additive** on Wi‑Fi. Per-peer native connect is usually **one TCP session** (LAN direct **or** bridge tunnel for calls).

```text
mDNS Discovered → dial_peer_tcp (LAN session)

mDNS Expired → remove_session; set_peer_on_local_lan false
  → new WAN call: bridge_request path
  → text: unchanged (delivery)

Device left LAN (mobile-data) → refresh_os_network_truth
  → coord register if public endpoint changes
  → mDNS state purged on LAN loss paths in network_transport / connect upkeep

Wi‑Fi return → notify_network_change → fresh ephemeral listen + mDNS publish
  → Discovered → LAN session again (calls)
```

**Healthy signs:** `mdns discovered` → `chat_ready` on LAN; `bridge request ok` → `wss connecting` for WAN calls; delivery logs for text independent of connect.

---

## Network truth — OS default route (authoritative)

**Problem:** After Wi‑Fi ↔ mobile-data toggle, `profile=lan` vs `profile=mobile-data` in `Native/flow` could stay wrong if inferred from `if_addrs` alone. Wrong profile breaks LAN kick timing and logging — not delivery text.

**Root cause:** Interface lists **lag** the OS default route. On Android, cellular interfaces often remain visible after Wi‑Fi is default.

### Layered model (truth first)

| Layer | Source | Used for |
|-------|--------|----------|
| **OS default transport** | Android `ConnectivityManager` + validated capability; Linux default route iface + `wl*` operstate | `has_active_lan()`, `on_mobile_data_path()`, `profile=` |
| **Wi‑Fi link** | Android TRANSPORT_WIFI; Linux operstate | `platform_wifi_linked`, LAN recovery when Wi‑Fi returns |
| **Interface hints** | `if_addrs` | Listen bind, logging — **not** primary mode switch |
| **Remote peer path** | mDNS `Discovered` / `Expired` | Whether **that contact** is on LAN |
| **Wire health** | connect session writer | Stuck call path vs live delivery text |

`Native/flow` (~30s) should show **`os=wifi|cell/...`** flipping within ~1s of a toggle.

### Code map

| File | Role |
|------|------|
| `p2p/network_transport.rs` | `OsNetworkSnapshot`, `merge_os_network_truth`, `refresh_os_network_truth` |
| `android_network.rs` | JNI on connectivity notify |
| `linux_network.rs` | Default route + Wi‑Fi operstate |
| `connect/worker.rs` | Upkeep calls `refresh_os_network_truth` |

### Rules agents must not violate

1. **`os default = cellular`** → treat as mobile-data path even if RFC1918/cellular addrs still appear in `if_addrs`.
2. **`os default = wifi/ethernet` + wifi link up** → eligible for LAN mDNS stack.
3. **Do not** probe the network from Makepad or gate ack/coord policy in the UI.
4. **Do not** infer “peer is on LAN” from local `profile=lan` alone — use mDNS for that peer.

---

## Asymmetric LAN ↔ WAN (one native session, parallel services)

**Scenario:** Desktop on Wi‑Fi; phone on mobile-data. **Text** still flows via **delivery**. **Calls** need **bridge** when there is no LAN mux writer.

When the phone leaves Wi‑Fi, the desktop may still show an old LAN session until mDNS `Expired` — call signaling on that session can stall while delivery text still works.

**Policy:**

- Treat mDNS **`Expired`** as authoritative for “peer off LAN” — drop LAN session; do not wait on long TTL caches.
- **Calls:** if `!writer_open_for_peer` and coord configured → bridge path (`outbound.rs`).
- **Do not** block delivery or bridge pending poll because LAN upkeep is rediscovering.
- Prefer **event-driven** session reopen (disconnect, expired, bridge connected) over fixed “wait N seconds” retries.
- The connect worker holds **one writer per peer**. If dual TCP links appear (LAN + public TCP), keep one mux, close the stale direct only, and keep the WAN path.

---

## Discovery

Typical flow ([GHAL_BOL_URI_SCHEME.md](GHAL_BOL_URI_SCHEME.md)):

1. Guest scans host QR → stores `public_key_hex`.
2. Both peers register on coord when they have publishable **public TCP** (optional for CGNAT-only).
3. LAN: mDNS discovers configured contacts → direct TCP.
4. WAN calls: bridge when no LAN writer.
5. text: delivery when configured.

**Coord lookup** (`GET /v1/peers/{public_key_hex}`) returns online peers with routable TCP endpoints — used when dialing a peer’s **public** address (and for diagnostics). It does **not** replace mDNS on LAN.

---

## LAN dial policy and ephemeral ports

**There is no stable LAN TCP port.** Each process start binds **`0.0.0.0:0`** (ephemeral port). LAN reachability comes from **live mDNS**, not remembered ports.

| Mistake | Effect |
|---------|--------|
| Re-dialing LAN from a cached candidate set on a timer | Stale port storms after peer restart |
| Deferring coord/WAN because stale LAN candidate exists | Blocked bridge/delivery unnecessarily |
| Port-ranking heuristics | Masks cache bugs |

**Correct model:**

| Path | Rule |
|------|------|
| **LAN connect** | `Discovered` → dial **that** host:port once (dial inflight guard) |
| **LAN expire** | `Expired` → remove session; clear LAN flag |
| **Candidate memory** | Event-driven add/remove only — **not** upkeep re-dial source |

**LAN re-discovery:** mDNS query interval must stay **short** (seconds) via project mDNS config. Fast LAN recovery must work when coord/delivery WAN paths are down.

**Wi‑Fi toggle recovery (summary):** connectivity notify → fresh ephemeral listen + mDNS republish + purge stale candidates; coord `schedule_register_presence_force` when listen/public IP changes. Do not destructive-restart mDNS every tick (port churn storm).

See [DESIGN.md](DESIGN.md) § dial strategy. No UI dial policy or RFC1918 guessing from coord rows.

---

## Event-driven async — avoid assumed timers (canonical)

Whenever policy waits on work with **unknown duration** (TCP connect, bridge WSS, HTTP register, mDNS discovery), use **workers + events**, not “sleep N then retry.”

1. **Worker** — owns the operation until success/failure (dial task, bridge accept, delivery poll).
2. **Policy** — reacts to events (session up → drain outbox; expired → clear LAN; HTTP ok → set registered).
3. **Timers** — guardrails only: storm throttles, heartbeat/register dedupe, bridge accept backoff, in-flight dial observation.

| Area | Worker | Policy reacts to |
|------|--------|------------------|
| LAN connect | `dial_peer_tcp`, mDNS forwarder | `Discovered`, connect result, `Expired` |
| WAN call | bridge request + WSS | pending poll, WSS open, mux ready |
| text | delivery_runtime | mailbox events (not connect poll) |
| Register | `coord_register_tick` | endpoint diff, failure, stale presence |
| Makepad | — | Poll **display only** |

**Anti-pattern:** upkeep loops that re-kick the same recovery without a new event.

---

## Multiple coord servers

Configure **`GHAL_BOL_COORD_URLS`** in `env/.env.development` / `.env.production` (JSON array or comma-separated).

| Action | Policy |
|--------|--------|
| **Register / heartbeat** | Every reachable coord URL in the list |
| **Lookup** | Try URLs **in order**; stop on first success for that attempt |
| **Reconnect** | After drop, repeat lookup from the first server |
| **Unreachable** | Keep retrying all entries; LAN mDNS unaffected |

Each URL is **HTTP(S) presence + bridge WSS on the same host**.

---

## Caching policy (canonical)

**Rule:** disk cache **only immutable user-owned data** (keystore, contacts, transcript, preferences). If staleness could break chat or connectivity, **fetch live**.

| Data | Source |
|------|--------|
| Peer public TCP dial targets | `GET /v1/peers/{public_key}` |
| LAN TCP port | mDNS events only |
| Bridge tokens | Short-lived; from bridge API responses only |

**In-memory OK:** storm throttles, bridge accept backoff, coord lookup backoff, mDNS candidate set for **expire/failover** — not timer re-dial.

**No disk cache** for coord dial addrs or bridge tokens.

**Forbidden:** upkeep LAN re-dial from frozen port lists; UI routing caches.

---

## Helper modules

| Path | Role |
|------|------|
| `connect/worker.rs` | Main loop: mDNS, TCP listen, bridge pending poll, upkeep |
| `connect/peer_session.rs`, `session.rs` | Sessions, mux, outbox/acks |
| `connect/lan_discovery.rs` | mDNS publish/browse |
| `connect/bridge_*.rs` | WAN call bridge client |
| `delivery_runtime.rs` | text |
| `coord_runtime.rs` | Register, heartbeat, lookup JSON |
| `p2p/network_transport.rs` | OS network truth |
| `dm_transport/` | Dial-address parsing helpers |

---

## Stable in-process surface

The UI calls `ghal_bol_core::host`. Do not rename without a version bump:

- `host::unlock` / `host::lock` / `host::start_network`
- `host::set_open_room` / `host::set_app_visible`
- `host::send_text` (WAN → delivery when configured)
- `host::poll_event` — UI refresh only
- Contact APIs → `register_dm_peer` / `sync_contacts`
- Coord: register, heartbeat, lookup ([COORDINATION_SERVER.md](COORDINATION_SERVER.md))

---

## Invariants (do not break)

| Invariant | Owner |
|-----------|--------|
| Ack kinds, `ref_id` rules | `msg_v1.rs`, [GHAL_BOL_DM_MSG_V1.md](GHAL_BOL_DM_MSG_V1.md) |
| Recipient sends acks; Makepad never | `connect/outbox_acks.rs`, [DESIGN.md](DESIGN.md) |
| text via delivery when URL set | `text_transport.rs`, `delivery_runtime.rs` |
| Guest scans host QR | `connect_invite_v1.rs` |
| No full connect stop on contact upsert | `register_dm_peer` |
| OS default route drives `profile=` | `network_transport.rs`, `android_network.rs`, `linux_network.rs` |
| Register when publishable endpoints **change** | `coord_runtime.rs` |
| mDNS LAN dials from **events** only | `connect/worker.rs`, `lan_discovery.rs` |

---

## AI handoff — common mistakes

1. **Ack policy in the UI** — forbidden.
2. **Mutual QR requirement** — guest-only host key is intentional.
3. **Inventing a second WAN discovery path when coord fails** — forbidden; retry coord + use delivery for text.
4. **Blocking text on native connect `chat_ready`** — text is delivery.
5. **Skipping coord register on Wi‑Fi because LAN works** — parallel stacks.
6. **`if_addrs`-only network mode** — use § **Network truth**.
7. **Caching coord or LAN dial targets on disk** — § **Caching policy**.
8. **Stable LAN port assumptions** — § **Ephemeral LAN TCP ports**.
9. **Expecting CGNAT peers to publish a dialable public TCP** — they use bridge (calls) and delivery (text).

---

## Logging — see the precise flow

Use App log `Native/flow` (~30s) plus tagged `connect`, `coord`, `bridge`, `delivery` lines. Intent-gated traces avoid roster floods.

**Journey (info-level milestones):**

| # | Milestone | Typical tag |
|---|-----------|-------------|
| 1 | Node up | `connect: TCP listen …` |
| 2 | Coord registered | `coord: registered …` |
| 3 | LAN peer found | `connect: mdns discovered …` |
| 4 | Session up | `chat_ready` / `PeerConnected` |
| 5 | WAN call bridge | `bridge: bridge request ok` → `wss connecting` |
| 6 | Text | delivery worker send/recv (separate from connect) |
| 7 | Call media | `/ghal-bol/call/*` substreams once call is up |

Never infer chat delivery from connect alone when delivery mode is on — confirm delivery worker state for text.

---

## Related documents

| Doc | Relationship |
|-----|----------------|
| [DESIGN.md](DESIGN.md) | Canonical product behaviour |
| [GHAL_BOL_DM_MSG_V1.md](GHAL_BOL_DM_MSG_V1.md) | Wire format and ack kinds |
| [GHAL_BOL_CONNECT_V1.md](GHAL_BOL_CONNECT_V1.md) | Connect + WAN bridge |
| [GHAL_BOL_DELIVERY.md](GHAL_BOL_DELIVERY.md) | text mailbox |
| [GHAL_BOL_URI_SCHEME.md](GHAL_BOL_URI_SCHEME.md) | Invites / QR |
| [COORDINATION_SERVER.md](COORDINATION_SERVER.md) | Coord API, bridge, deploy |
| [PREMIUM_SERVICES.md](PREMIUM_SERVICES.md) | Optional Tier 3 paid relay (not shipping on coord1) |
