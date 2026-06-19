# Transport — libp2p data plane

**Status:** **libp2p is the production P2P transport.** A prior plan to replace libp2p with a custom native QUIC/TCP stack was **evaluated and discarded** (May 2026). This document is the canonical reference for how peers connect today.

**For AI / new sessions:** Read [AGENTS.md](../AGENTS.md) and [DESIGN.md](DESIGN.md) first. Transport changes must **not** move ack policy, outbox, or transcript merge into Flutter. **Start here for connectivity:** § **Connectivity lifecycle** → § **Network truth** → § **Parallel LAN + WAN** → § **Asymmetric LAN↔WAN mux recovery**. Transport reachability is **live-only** — see § “Caching policy (canonical)”.

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

## Parallel LAN + WAN transport (2026-06-17 — canonical)

**Policy:** LAN and WAN **always run in parallel** at the node level and per peer. They are not mutually exclusive modes.

### Both links active (product requirement)

When peer **A** and peer **B** are connected over **LAN and WAN at the same time**, **both paths stay up and keep doing their job**:

| Path | Job while connected |
|------|---------------------|
| **LAN** (direct TCP via mDNS) | Local discovery, low-latency direct reachability, immediate LAN handover when both are on Wi‑Fi |
| **WAN** (coord + relay circuit) | Coord phone-book presence, remote reachability, relay circuit for CGNAT peers, **instant failover** when LAN drops |

**Required behaviour:**

- **Keep both libp2p connections** — never `close_connection` on relay because direct LAN appeared, and never stop coord/reserve/mDNS because the other path succeeded.
- **Both may dial in parallel** — throttled only to prevent storms, not to pick “one winner.”
- **WAN stack stays on on Wi‑Fi** — coord register/poll, relay reservation, and per-peer circuit dials continue even when mDNS shows the contact on LAN.
- **LAN stack stays on when WAN is up** — mDNS browse, ephemeral listen, and mDNS-driven dials continue even when relay is connected.

**One chat mux (wire constraint, not “one path”):** libp2p carries at most **one** live `/ghal-bol/msg/1.0.0` stream per contact at a time. That is a **mux** limit, **not** permission to tear down the other link. While the mux is on one connection, the **other link stays connected** (keepalive ping, inbound accept, ready for stream reopen on failover). When opening a stream and **both** links exist, attach on direct first (lower latency) — the relay link **remains established**.

**Wrong docs/code (regressions):** treating WAN as “idle backup,” closing relay when direct connects, deferring coord lookup while LAN is up, **`coord_lookup_dm_peer` skipping lookup when `peer_on_local_lan`**, DCUtR hole-punch while coord is configured (blind multi-dial from identify), treating a LAN-only mux as stable while the peer is off-LAN without a relay link, or implying only one stack should run when both are reachable.

| Layer | LAN | WAN | Relationship |
|-------|-----|-----|--------------|
| **Infrastructure** | mDNS browse + ephemeral TCP listen | coord relay bootstrap + `/p2p-circuit` reserve + coord register/poll | **Both always on** on Wi‑Fi; WAN-only on mobile-data/CGNAT |
| **Per-peer links** | Direct TCP when mDNS discovers the contact | Relay circuit when coord lookup succeeds | **Both may be connected simultaneously** — each doing its job; do not tear down either because the other succeeded |
| **Discovery / dial** | mDNS `Discovered` → explicit LAN TCP multiaddr | coord lookup → explicit `/p2p-circuit` multiaddr | **Both may dial** the same peer — **independent throttles** (`lan_dial_last_ms` vs `circuit_coord_dial_last_ms`); neither path gates the other |
| **Dial policy** | Live mDNS addr only — **no blind peerstore / identify dials** | Live coord circuit addr only — **no bare relay bootstrap TCP** | **DCUtR disabled** when coord is configured — no hole-punch multi-dials from stale identify addrs |
| **Wire (mux)** | Hosts DM stream when attached | Same protocol — may also host stream on failover | **One mux at a time**; **both links stay up**; stream attach prefers direct when both exist |
| **Application state** | — | — | **Single source of truth in Rust** — see [DESIGN.md](DESIGN.md) § “Unified message state (E)” |

**Independence rule:** LAN health must **not** suppress WAN dials (and vice versa). Skip redundant relay **dial** only when a **relay** link is already connected **and** carries a stable DM stream — not merely because direct LAN is up. WAN recovery must **not** block LAN listen/mDNS upkeep.

**What parallel does *not* mean:** uncoordinated dial **spam** (many `swarm.dial`/s for the same peer per second) or a second transcript/outbox in Flutter. Throttles and `PeerCondition::NotDialing` prevent storms; **all** message/ack/delivery state lives in one Rust store.

**Handover:** When a peer leaves LAN, WAN is **already connected and active** — fallback is immediate. When mDNS discovers a peer on LAN, add the direct link **without** closing the relay link. Stream reopen may attach on direct when both links exist; relay stays up.

**Supersedes:** older “defer coord relay while LAN in flight” / “close relay when direct connects” / “WAN is warm backup only” guidance in this file and DESIGN.md dial sections — those caused split-brain during LAN↔WAN transitions.

---

## Stream-first symmetric connect (wire layer)

**Canonical model** — documented in [DESIGN.md](DESIGN.md) § “Stream-first symmetric connect”. Ghal Bol: one live DM stream per contact, ~1s `dm_upkeep`, stream reopen on mux failure without tearing down libp2p links. Coord + relay + mDNS run **in parallel** as discovery inputs; the wire layer still has **one mux per contact**.

```text
Per contact (every ~1s dm_upkeep):
  if live DM stream writer (dm_peer_stream_up):
    noop — no coord lookup, no disconnect, no identify dial
  else if not libp2p-connected:
    LAN + WAN may both dial (parallel, throttled) — first success wins
    open /ghal-bol/msg/1.0.0 OR accept inbound on same handler
  else if connected but no stream:
    open_stream once — attach on direct when both LAN + relay links exist (both links stay up)
  if stream up && outbox pending → drain
```

| Principle | Implementation |
|-----------|------------------|
| Both listen | Swarm listens; inbound streams accepted on `/ghal-bol/msg/1.0.0` |
| One stream per contact | Stream writer map keyed by peer / `public_key_hex`; `dm_peer_stream_up` → upkeep noop |
| Symmetric | Outbound `open_stream` and inbound accept use the same handler — no fixed caller/listener role |
| Send = connect | `send_text_dm` / outbox retry share the stream-first path; hub room open not required |
| Parallel transport | mDNS + coord lookup + relay reserve run concurrently; `dm_upkeep` triggers WAN when stream is down |
| Dual links active | Keep relay + direct connections when both exist — both ping keepalive; do not `close_connection` on LAN upgrade |
| No DCUtR with coord | `dcutr` behaviour off when coord configured — WAN = explicit `/p2p-circuit` dial only; LAN = mDNS explicit TCP; prevents `hole punch` storms on handover |
| Stale LAN mux | Peer off LAN without relay link → `dm_peer_chat_link_stable` false + stream reopen — coord circuit dial (acks/ticks use live path) |

**Discovery vs wire:** coord `GET /v1/peers`, relay circuit multiaddrs, and mDNS are **parallel discovery inputs**. Duplicate frames on the single DM stream are deduped in Rust (`append_if_new`); duplicate acks merge monotonically in `dm_transcript_store` (read ⊃ delivered).

**Latency target:** seconds to `peer_connected` + `chat_ready` when the remote has finished WAN registration (phases A–D in § “WAN prerequisites”) — matching the original build’s feel.

---

## libp2p stack (current)

Enabled in `ghal_bol/Cargo.toml` and wired in `chat_server.rs`:

| libp2p piece | Role in Ghal Bol |
|--------------|------------------|
| **QUIC / TCP + Noise + Yamux** | Encrypted connections and multiplexing |
| **`libp2p-stream` `/ghal-bol/msg/1.0.0`** | Framed DM channel for `ghal_bol_msg_v1` |
| **mDNS** | LAN discovery of configured peers |
| **Relay + DCUtR** | NAT traversal — reserve a circuit on a **Ghal Bol relay** (co-located with a configured coord server). **With coord configured, DCUtR is disabled** — stream-first uses explicit mDNS LAN + coord `/p2p-circuit` dials only (no identify hole-punch). **WAN peer discovery does not use Kademlia or public libp2p bootstrap peers** — see § “Connectivity lifecycle” |
| **AutoNAT, UPnP, Identify** | Reachability and peer metadata |
| **Ping** | **Connection keepalive** — periodic pings keep an idle DM/relay link active so it is not dropped by `idle_connection_timeout`; also detects a dead route faster (`PING_INTERVAL_SECS` 10s < idle timeout) |
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

This section encodes the **binding connectivity rules** — they take precedence over any other guidance here. The whole transport must behave this way; if any other doc disagrees, it is wrong and must be updated to match this.

1. **Start on first unlock, run in the background.** After the user unlocks the first time, the node (`ghal_bol_daemon` / Android `:p2p`) starts and **keeps running** regardless of UI state. UI lock/suspend never stops the node, poll, or ack loops.
2. **Watch the network continuously.** OS connectivity callbacks (`notify_network_change`) plus `refresh_os_network_truth` on each `network_tick` (~1s) read **default route transport** and internet validated — not `if_addrs` alone. Loss or change triggers recovery without user-visible disruption (`handle_network_path_change`, `run_wan_recovery_pass`). See § **Network truth**.
3. **Find both addresses fast.** The node determines its **LAN** address (interface scan / mDNS) and its **globally reachable** address (public listen, AutoNAT/UPnP, or a relay `/p2p-circuit` when behind NAT/CGNAT).
4. **Register at coord as soon as a reachable address exists, and keep it fresh.** Once a publishable global endpoint (public TCP or relay circuit) is known, register with `ghal_bol_server`. **Re-register** on endpoint change, failed register, relay reservation accepted, handover, or stale presence — **not** on every heartbeat tick when endpoints are unchanged (`should_throttle_register` in `coord_runtime.rs`; details in [COORDINATION_SERVER.md](COORDINATION_SERVER.md) § **Client register & heartbeat policy**).
5. **WAN must always work when both peers have internet and coord is reachable.** This is the baseline guarantee — coord lookup + relay reservation + DCUtR, never gated off by being on Wi‑Fi/LAN.
6. **LAN is per-peer and additive.** Use the LAN path **only** for a contact actually discovered on the local LAN (mDNS), never globally. If the LAN is lost, that peer transparently falls back to the normal WAN path with no user-visible change.
7. **Coord down ≠ app offline, but WAN discovery pauses.** When coord is unreachable, **do not** fall back to Kademlia DHT or public libp2p bootstrap peers for WAN peer discovery (§ “Connectivity lifecycle”). **LAN (mDNS) still works** for contacts on the local network. Keep retrying **all configured coord servers** at a regular interval — never stop. **WAN requires coord + relay** when both peers have internet.
8. **Internet/coord recovery is immediate** — when internet or coord comes back, the continuous watch detects it within seconds and resumes WAN registration/lookup across the coord list without a libp2p restart.
9. **Registration truthfulness.** Client `coord_registered` must match what coord actually lists — verified via `GET /v1/peers/{self}` (relay presence poll for CGNAT-only peers). Never treat the client as registered when HTTP self-lookup or the relay live gate disagrees.
10. **Network switch readiness (Wi‑Fi ↔ mobile ↔ other).** Before the user sends again after a path change, `:p2p` must: re-register when **publishable endpoints change**, clear coord lookup backoff, mark urgent reconnect where streams are down, and reopen DM chat streams so delivery/read acks and outbox drain resume without user action.
11. **No silent stall on a dead mux.** A libp2p link that looks connected but whose `/ghal-bol/msg/1.0.0` stream died must trigger stream reopen and retry queued `ack_received`, `ack_read`, and outbox rows — not idle until a timer fires (see § “Steady connection”, stream mux recovery).
12. **`:p2p` / daemon restart.** On `p2p_start` or service restart, re-fetch relay (`GET /v1/relay`), re-reserve when needed, and re-register when endpoints are known. Do not trust in-memory registration from a prior process.

**UI vs native (do not break):** `ghal_bol` owns ack policy, outbox, delivery/read ticks, contacts, transcript, dial, and coord/relay. `ghal_bol_ui` reports only app visible + open room via **`GhalBolUiSession`** → `p2p_sync_ui_session`. Poll / `peer_connected` / `isStreamReady` in Flutter are **display hints only** — never gate sends or acks in Dart. See [DESIGN.md](DESIGN.md) § `GhalBolUiSession`.

The ultimate goal is **strong, reliable, smooth** peer interaction. coord + libp2p are sufficient for this across WAN and LAN; do not regress these guarantees for performance or simplicity.

---

## End-to-end WAN phases (both peers — read before changing transport)

WAN chat is a **pipeline with hard gates**. Each device must complete phases A→D before the **other** device can dial it. Opening a chat room or sending a new message does **not** substitute for these gates; `:p2p` drives them from `p2p_start`, outbox restore, `dm_upkeep`, and `coord_tick`.

```text
Per device (Android :p2p / Linux daemon)
──────────────────────────────────────
A. Swarm up          p2p_start, dm_peers registered
B. Relay bootstrap   TCP to GET /v1/relay peer (log: bootstrap connection)
C. Own reservation   listen_on(…/p2p-circuit) after Identify on HOP
                     (log: reservation accepted, relay listen addr)
D. Coord presence    Client `POST /v1/register` with **public routable IPv4 TCP only** (peer’s own inbound DM listen);
                     never LAN, never relay bootstrap, never `/p2p-circuit`.
                     Relay server upserts `/p2p-circuit` on reservation (identify `;pk=`).
                     CGNAT-only clients poll `GET /v1/peers/self` until circuit visible.
                     (log: coord registered or relay presence visible, server: peer registered)

Cross-device (after BOTH at phase D)
────────────────────────────────────
E. Coord lookup      GET /v1/peers/{remote_pk} → /p2p-circuit multiaddr
F. Circuit dial      swarm.dial(peer’s circuit addr via coord relay)
G. Stream + outbox   ConnectionEstablished → /ghal-bol/msg/1.0.0 → resync outbox
```

**Android vs Linux transport:** Linux builds TCP+QUIC+Noise; Android `:p2p` is **TCP+Noise only** (no libp2p DNS transport — coord expands `/dns4/…/p2p-circuit` to `/ip4/…` aliases at register). Both use the same relay-client state machine in `chat_server.rs`.

### Phase-gate symptoms (from App log)

| Log pattern | Phase stuck | Meaning |
|-------------|-------------|---------|
| `waiting for relay/public listen endpoint before coord register` | B or C | No publishable WAN addr yet — reservation not accepted |
| `reservation accepted` but no `coord registered` | D | Relay OK; client register failed and relay presence poll did not see circuit yet |
| `register HTTP 400` … `relay circuit endpoints are registered by the relay server` | D | Client must not POST `/p2p-circuit` — server owns circuit presence on reservation |
| `register — reason=coord register HTTP transport failed` | D | **HTTPS** to coord broken (VPN/DNS/TLS) — not libp2p |
| `lookup — category=peer_not_on_coord` (404) | Remote still A–D | **Expected** until remote finishes pipeline; outbox waits; lookups stop after first 404 until real disconnect (`mark_dm_reconnect_urgent`) |
| `lookup — category=coord_http_unreachable` | Coord HTTP flap (ngrok/VPN/DNS) | **15s** per-peer transport throttle when `coord_http_degraded`; urgent reconnect does **not** bypass transport throttle; 404 clears degraded when HTTP responds |
| `awaiting_coord_mirror` + local `relay_circuit=true` | D | Relay presence poll: 6s fast (500ms) then up to 60s slow; nudges `notify_relay_refresh` + `POST /v1/register` when public TCP available |
| `issue=no_dial_addrs \| reason=no dial addrs — coord has no record` | E blocked by remote D | Same as 404 — do not treat as “dial broken” |
| `coord_lookup_peer ok — dialing` but no `dm peer connected` | F | Circuit dial failing — check server `circuit ACCEPTED/DENIED`; check throttles not blocking all paths |
| `mdns dialing` + `coord dialing` same peer on Wi‑Fi | OK (if throttled) | **Parallel transport** — both paths expected; unhealthy only if **no** `dm connection established` for minutes |
| `mdns dialing` only for 15s+ then relay (no parallel race) | F (LAN path) | LAN TCP failing (firewall/wrong port); relay should follow after in-flight window — expected sequence |
| `issue=… ResourceLimitExceeded` | F | Relay server rate limiters — redeploy `ghal_bol_server` |
| `dm peer connected` + `outbox resync` | G ✓ | Pending transcript outbox drains — no new user send required |

---

## Hybrid coord presence (WAN directory + relay circuit)

**Problem this solves:** CGNAT/mobile peers cannot publish a dialable public TCP port. They still need a **coord phone-book entry** so the other device can `GET /v1/peers/{pk}` and dial a `/p2p-circuit` multiaddr. Posting the circuit from the client was fragile (400 storms, wrong addrs, heartbeat blocking the swarm thread).

**Model (shipping): split who owns what on coord**

| Endpoint type | Who registers it | How |
|---------------|------------------|-----|
| **Public TCP** (`tcp://routable-ip:port`) | **Client** | `POST /v1/register` when the device has a **globally routable inbound** DM listen (UPnP / port-forward / public IP). Must be the peer’s own socket — **not** the coord relay bootstrap host:port from `GET /v1/relay`, **not** RFC1918/CGNAT LAN. Multiaddr must end with `/p2p/<local_peer_id>`. |
| **LAN TCP** (`tcp://192.168.x:port`) | **Never on coord** | Same-subnet peers use mDNS direct TCP only — not in `POST /v1/register`. |
| **`libp2p` `/p2p-circuit/…`** | **Relay server** | On `reservation ACCEPTED` + identify `agent_version` `ghal_bol/<ver>;pk=<66-hex>`, `ghal_bol_server` upserts the circuit into SQLite (`presence.rs`). Clients **must not** POST `/p2p-circuit` (server returns 400). |

**Server lifecycle (coord must enforce):**

1. **`POST /v1/register`** — accept only client **public routable IPv4 TCP** endpoints that are the peer’s own inbound listen. Reject `/p2p-circuit`, RFC1918, CGNAT-only, and **relay bootstrap** addresses (`/ip4/<relay>/tcp/<port>` from `GET /v1/relay` is for dialing the relay, not registering yourself).
2. **Relay reservation accepted** — server **adds** the peer’s `/p2p-circuit` row (`upsert_relay_circuit`). This is how CGNAT/mobile peers become WAN-dialable.
3. **Relay reservation ends** — server **removes only** the `/p2p-circuit` endpoint (`remove_relay_circuit`). If the peer still has a valid public TCP row from `POST`, **keep** that row. Delete the SQLite row only when **no** WAN-dialable endpoints remain.
4. **Lookup** — WAN dials use `/p2p-circuit` multiaddrs only (`filter_coord_dial_addrs` in the client). A coord row with only relay bootstrap `tcp://159.223.x:28048` is **undialable** (`peer_on_coord_no_dial_addrs`). **Live gate (2026-06-19):** `GET /v1/peers` returns `/p2p-circuit` only while the relay still holds an **accepted reservation** for that peer (`relay_live.rs` + `RelayLiveRegistry`). SQLite may retain a circuit row after happy-eyeballs `ReservationClosed`; lookup strips it until `ReservationReqAccepted` again — dialers get **404** instead of **200 + Timeout**.

**Client files:** `coord_runtime.rs` (`endpoints_for_coord_register`, `schedule_coord_presence_after_relay`, `promote_relay_presence_if_visible`), `chat_server.rs` (reservation hook, WAN recovery). **Server files:** `relay.rs`, `agent_pk.rs`, `presence.rs`, `routes.rs`.

**CGNAT-only path (phone on mobile data):**

1. Reserve circuit on coord relay (phases B–C).
2. Server writes `/p2p-circuit` to coord on reservation.
3. Client has **no** public/LAN tcp for `POST /v1/register` → `schedule_coord_presence_after_relay()` polls `GET /v1/peers/{self}` until circuit visible → sets `coord_registered=true` without blocking libp2p (no synchronous heartbeat `join` on the swarm thread).

**ngrok dev:** coord HTTP client sends `ngrok-skip-browser-warning` + `Accept: application/json` on all requests (`coord.rs`). Lookup errors include response body snippets when JSON parse fails (ngrok HTML interstitial).

**Do not regress:**

- Client `POST` of `libp2p` circuit endpoints (400 + register storm).
- Registering **relay bootstrap** TCP (`/ip4/<relay-host>/tcp/<relay-port>/p2p/<relay_peer>` or bare relay IP:port) as your own endpoint — not a DM listen socket.
- Dropping WAN relay circuits on mobile-data because stale direct tracking still thinks “direct LAN up” after `left LAN` — use **`dm_direct_conn_ids`**, not `peers_direct_conns` alone; `prefer_direct_dm_path_over_relay` is **stream preference only**, not permission to close relay links on mobile-data.
- **Closing relay links when direct LAN connects** — parallel transport keeps **both links active**; regression causes WAN handover stalls. Stream attach may prefer direct when both exist — never `close_connection` on relay when direct is up.
- **`coord_dial_from_lookup_addrs` stripping relay addrs** when `peer_has_direct_connection` — blocks WAN backup on Wi‑Fi; use additive relay dial instead.
- **Blocking LAN upkeep during WAN recovery** — `lan_handover_upkeep_if_needed` runs **in parallel** with `ensure_wan_relay_circuit` (no early return). `try_recover_lan_after_wifi_available` must **not** treat `wan_recovery_active && !relay_circuit_listening` as `relay_lost_on_lan` (dev coord/ngrok down would purge mDNS every 5s and LAN never discovers).
- **`lan_handover_upkeep` kicking LAN for WAN-only roster peers** — offline contacts never seen on mDNS must not trigger `link down, no mDNS candidate yet` every 5s (churns ephemeral listen + `notify_stream_reopen`, breaks relay to mobile-data peers). Gate on **`peer_eligible_for_lan_handover`**: prior LAN/mDNS sighting (`peer_on_local_lan` / mDNS candidate / rediscovery requested), or **pending outbox on Wi‑Fi** when the peer was previously on LAN — **not** bare hub foreground room for a WAN-only contact (mobile data). Never all roster peers. Ghost `peer_not_on_coord` contacts with no outbox and no LAN history are WAN-only.
- **`dial_mdns_lan_addr` clearing `circuit_dial_in_flight`** — LAN must not cancel WAN in-flight tracking; use `NotDialing`, parallel dial (§ Parallel LAN + WAN).
- Re-issuing `listen_on(/p2p-circuit)` on every libp2p “listener closed cleanly” during an in-flight re-reserve (renewal window).

---

## WAN coordination (peer ↔ coord ↔ relay — 2026-06-17)

**Problem:** Three layers can disagree: libp2p shows `relay_circuit=true` and `coord_registered=true` locally while coord HTTP still returns a `/p2p-circuit` that the relay rejects with `NoReservation`. Symptom: `coord_lookup_peer ok` → dial fails → minutes until re-reserve.

**Model:** One authoritative owner per fact; clients never invent WAN dialability.

| Fact | Owner | Client behaviour |
|------|--------|------------------|
| `/p2p-circuit` row on coord | **Relay server** on `ReservationReqAccepted` + identify `pk=` | Never `POST` circuit; poll `GET /v1/peers/{self}` until visible (`schedule_coord_presence_after_relay`) |
| Circuit row removed | **Relay server** on `ReservationTimedOut` / `CircuitReqDenied(NoReservation)` / `ConnectionFailed` when dst not live | Bootstrap TCP drop: `mark_coord_relay_hop_lost` (keeps `coord_registered` while local circuit listen up) + `ensure_wan_relay_circuit` |
| Lookup circuit visibility | **Relay live registry** (`RelayLiveRegistry`) | Stale SQLite circuit rows hidden on `GET` until reservation live again |
| `coord_registered` (client flag) | **Client** only after HTTP self-lookup shows WAN-dialable endpoint **and** local IPv4 relay circuit is listening (CGNAT) | `refresh_relay_presence_from_coord` clears flag if coord lists circuit but local listen is down |
| Remote peer stale circuit | **Dial outcome** | `NoReservation` → `note_remote_peer_circuit_stale` (urgent re-lookup, no 404 backoff) |

**Phase labels** (`Native/flow` `wan_coord=` — `wan_coord.rs`):

```text
awaiting_relay_bootstrap   — no coord-relay TCP
awaiting_relay_circuit     — bootstrap up, no IPv4 /p2p-circuit listen
awaiting_coord_mirror      — circuit listening, coord self-lookup not yet WAN-ready
wan_ready                  — circuit + coord_registered + HTTP healthy
http_degraded              — registered but coord HTTP failing (transport throttle)
```

**Event-driven sync (no timer-based “wait N seconds”):**

- `sync_wan_coord_local_snapshot` on bootstrap connect/close, reservation accepted, relay `NewListenAddr`, flow snapshot tick.
- Bootstrap HOP lost → immediate WAN recovery + `schedule_coord_presence_after_relay` (do not keep dialing stale coord circuits).
- **Left LAN** (`lan` → mobile-data/CGNAT) → `wan_coord::on_left_lan()` — purge mDNS, refresh own phases B–D, phase E–F for all DM peers; **keep relay libp2p links** (parallel transport). Close **direct** `ConnectionId`s only (`close_direct_dm_connections`).
- **LAN restored** (mobile-data → Wi‑Fi) → `wan_coord::on_lan_path_restored()` — mDNS rediscovery + **keep relay reservation warm** (`ensure_wan_relay_circuit`); WAN relay links unchanged.
- Remote peer off LAN → `wan_coord::on_peer_off_local_lan(pk)` — urgent coord lookup + stream reopen on existing relay; close direct links only.
- LAN handover: do not churn relay for WAN-only roster peers (`peer_eligible_for_lan_handover`); when LAN upkeep needs full kick but a relay **circuit dial is in flight**, queue full kick and run **soft** interim only — § “Deferred full LAN kick”.

**Server (`relay.rs`):** reservation refcount; `end_reservation` → `remove_relay_circuit` on coord; `NoReservation` on inbound circuit dial purges destination presence.

**Do not regress:**

- Setting `coord_registered=true` while local relay circuit is not listening (CGNAT).
- Trusting coord lookup after `NoReservation` without urgent re-lookup.
- Clearing coord presence on relay **client disconnect** (happy-eyeballs churn) — only reservation end events.
- Client `POST` of `/p2p-circuit` (still 400).

---

## LAN ↔ WAN handover (both directions — verified 2026-06-18)

**Policy:** LAN and WAN are **additive and parallel**, not replacements. Wi‑Fi runs mDNS + coord/relay **at the same time**. Per-peer: both direct and relay links may stay up; one DM stream; unified Rust store for all frames/acks.

```text
On LAN (mDNS Discovered)
  → direct TCP dial / upgrade (additive — relay link stays connected)

Peer leaves LAN (mDNS Expired, last candidate gone)
  → forget_peer_on_local_lan immediately
  → close direct ConnectionIds only (relay circuits stay up)
  → wan_coord::on_peer_off_local_lan + coord lookup + stream reopen

left LAN / mobile-data (full handover)
  → purge mDNS LAN state; keep coord heartbeats where possible
  → WAN recovery: bootstrap + reserve + hybrid presence
  → do NOT drop peer relay link because direct counter was stale

Wi‑Fi return
  → kick_lan_dm_rediscovery (fresh ephemeral TCP + mDNS restart)
  → mDNS Discovered → direct path added alongside WAN (seconds)
```

**Wire layer unchanged:** one DM stream per contact; `dm_upkeep` owns stream reopen. **Transport layer:** LAN + WAN discovery/dial run in parallel — see § “Parallel LAN + WAN transport”.

**Log signatures of healthy switching:**

| Leg | Healthy signs |
|-----|----------------|
| **LAN** | `mdns discovered` → `dm connection established … (direct)` → `chat_ready` / `stream=true` |
| **WAN** | `reservation accepted` → `relay listen addr` → `coord registered` or `relay presence visible` → `coord_lookup_peer ok — dialing` → `dm connection established … (relay)` |
| **LAN return after WAN** | `mdns discovered` on new ephemeral port &lt; few s → `conn=true,stream=true` without manual restart |

**Known noisy but OK during handover:** `LAN DM rediscovery — link down, no mDNS candidate yet` briefly before `mdns discovered`; ephemeral TCP port change every handover (by design — see § “Ephemeral LAN TCP ports”).

---

## Network truth — OS default route (authoritative)

**Problem (observed 2026-06-19):** After Wi‑Fi ↔ mobile-data toggle, `profile=lan` or `profile=mobile-data` in `Native/flow` could stay **wrong for seconds or minutes**. P2P then routed acks/outbox on the wrong path (zombie LAN mux, relay reset loops, `outbox_pending` stuck, ticks missing on WAN).

**Root cause:** `if_addrs` (interface names + IP addresses) **lags** the OS default route. On Android, `rmnet` / CGNAT addresses often **remain visible** after Wi‑Fi is default. Promoting `profile=lan` from a libp2p RFC1918 listen addr **without** OS Wi‑Fi confirmation made the lag worse.

**No third-party network library is required.** The OS already exposes truth; Rust must read it.

### Layered model (truth first)

| Layer | Source | Typical latency | Used for |
|-------|--------|-----------------|----------|
| **OS default transport** | Android: `ConnectivityManager.getActiveNetwork()` + `hasTransport(WIFI\|CELLULAR\|ETHERNET)` + `NET_CAPABILITY_VALIDATED`. Linux: `/proc/net/route` default iface → classify `wl*` / `wwan` / `eth` + operstate | **Immediate** on Android connectivity callback; **~1s** on `network_tick` | `has_active_lan()`, `on_mobile_data_path()`, `profile=lan` vs `mobile-data`, `handle_network_path_change` |
| **Wi‑Fi link** | Android: any registered network with `TRANSPORT_WIFI`. Linux: `/sys/class/net/wl*/operstate` | Same as above | `platform_wifi_linked`, LAN kick when Wi‑Fi returns while cellular is still default |
| **Interface hints** | `if_addrs` crate | Often **30–120s** after toggle | RFC1918/CGNAT addresses, listen bind, logging only — **not** primary mode switch |
| **Remote peer path** | mDNS `Discovered` / `Expired`; coord `GET /v1/peers` | Event-driven | Whether **that contact** is on LAN vs WAN — **not** derivable from local OS profile alone |
| **Wire health** | `dm_peer_stream_up`, outbox drain, `dm_wire_activity_ms` | Event-driven | Zombie mux detection when stream is up but peer path is wrong |

`Native/flow` (~30s) must show **both**:

```text
profile=mobile-data os=cell/validated/wifi_down …
profile=lan os=wifi/validated/wifi_up route=wlan0 …
```

After a toggle, **`os=` should flip within ~1s** (callback) or on the next tick — not minutes after `profile=`.

### Code map

| File | Role |
|------|------|
| `p2p/network_transport.rs` | `OsNetworkSnapshot`, `LocalNetworkProfile`, `merge_os_network_truth`, `refresh_os_network_truth`, `network_handover_key` (includes `os_default` + `os_validated`) |
| `android_network.rs` | JNI probe on connectivity notify; `:p2p` Kotlin registers `NetworkCallback` only (no Flutter RPC) |
| `linux_network.rs` | Default IPv4 route iface + `wl*` operstate |
| `p2p/chat_server.rs` | `network_tick` calls `refresh_os_network_truth` then `handle_network_path_change`; `effective_network_profile` requires `platform_wifi_linked` before promoting LAN from listen addrs |

### Rules agents must not violate

1. **`os default = cellular`** → `has_active_lan() = false` and `on_mobile_data_path() = true` **even if** `rmnet` / RFC1918 still appear in `if_addrs`.
2. **`os default = wifi/ethernet` + wifi link up** → eligible for `profile=lan` (mDNS + direct TCP stack runs).
3. **Do not** call `p2p_notify_network_change` from Flutter — Android `:p2p` + Linux `network_tick` own handover.
4. **Do not** infer “peer is on LAN” from local `profile=lan` alone — use mDNS for that peer or coord circuit for WAN.
5. **Internet validated** (`NET_CAPABILITY_VALIDATED` / route operstate) is logged and used for urgency; brief `unvalidated` right after Wi‑Fi associate is normal — handover still keys off **default transport**.
6. **Flutter `NetworkHelper`** (`ghal_bol_ui/lib/network_helper.dart`) — thin poll loop for **UI display only**. OS truth from **`ghal_bol`** via daemon RPC `network_snapshot` (`:p2p` / `ghal_bol_daemon` → `android_network` / `linux_network`). Linux in-process FFI fallback only when daemon is off. Rust logs: `Native/network` `ui snapshot …`; Flutter App log tag **`Network`**. **Never** gates P2P dial/ack/coord policy in Dart.

### Network-truth regressions

| Wrong approach | Symptom | Correct behaviour |
|----------------|---------|-------------------|
| `if_addrs` only for `profile=` | Minutes on wrong mode after toggle | `refresh_os_network_truth` + merge on every tick/notify |
| “cellular iface present” ⇒ mobile-data while OS default is Wi‑Fi | WAN deprioritized on phone still on Wi‑Fi | `os_on_cellular_default()` gates mobile path |
| `effective_network_profile` from listen addr alone | `profile=lan` after leaving Wi‑Fi | Require `platform_wifi_linked` (OS truth) |
| `peers_direct_conns` counter for stale mux | `reopen … peer off LAN` every 1s; relay ACCEPTED/closed loop on server | Track **`dm_direct_conn_ids`**; `close_direct_dm_connections` keeps relay |
| `swarm.disconnect_peer_id` on relay stream open timeout | Good relay torn down; `Pending connection aborted` | `apply_pending_dm_link_resets`: reopen on relay or drop **direct** only |
| Reconcile before link reset on wrong tick order | Stream reopens on dead LAN before disconnect | `reconcile_all_stale_lan_mux_for_wan` **before** `apply_pending_dm_link_resets` in `dm_upkeep` |

---

## Asymmetric LAN↔WAN mux recovery (one stream, parallel links)

**Scenario:** Desktop on Wi‑Fi (`profile=lan`); phone on mobile-data (`profile=mobile-data`). Both may have **relay + direct** libp2p connections at once (§ “Parallel LAN + WAN”). libp2p allows **one** live `/ghal-bol/msg/1.0.0` writer per contact — if that mux sits on a **dead direct** path, the app shows `conn=true,stream=true` while acks and outbox go nowhere.

### Detection (`chat_server.rs`)

| Check | Meaning |
|-------|---------|
| `peer_has_live_mdns_lan(peer)` | Remote currently on **our** LAN (mDNS) |
| `dm_direct_conn_ids[peer]` non-empty | We have live **direct** `ConnectionId`s |
| `peer_has_relay_connection(peer)` | We have live **relay** `ConnectionId`s |
| `dm_peer_needs_wan_relay_path` | Need WAN mux recovery: off-LAN without relay, **or** relay **and** direct both up (asymmetric) |
| `dm_peer_chat_link_stable` false | Upkeep may coord-dial / reopen even if `swarm.is_connected` |

### Recovery (event-driven, throttled)

On each `dm_upkeep` tick (~1s), **in order:**

1. `reconcile_all_stale_lan_mux_for_wan` — 5s throttle per peer; skip if not connected
2. `apply_pending_dm_link_resets` — if relay up: drop **direct** mux or reopen stream on relay; full disconnect only when no relay
3. `upkeep_dm_peers` — coord/LAN dial; `should_defer_stream_open_for_wan_mux` blocks opening stream on stale direct before direct links close

**Healthy logs after phone leaves Wi‑Fi:**

```text
reopen … peer off LAN; recover WAN mux for acks/outbox   ← at most ~once per 5s per peer
close direct DM link … (ConnectionId) — relay kept
dm connection established … (relay)
chat_ready / frame on wire / outbox_pending drops
```

**Broken loop (do not ship):**

```text
reopen … peer off LAN   ← every 1s
reset DM link … chat stream open timed out
dm peer disconnected
(relay server: circuit ACCEPTED → closed cleanly every ~3s)
```

### libp2p conflicts that drive this

| libp2p behaviour | Ghal Bol impact | Mitigation |
|------------------|-----------------|------------|
| Inbound/outbound relay reports as `/p2p/<peer>` (not `/p2p-circuit`) — [#5741](https://github.com/libp2p/rust-libp2p/discussions/5741) | Misclassified “direct” → false `dm_direct_conn_ids`, mux reconcile loop | `dm_relay_circuit_pending` on circuit events; bare `/p2p/<peer>` with coord → **relay** (LAN direct always has `/ip4/…/tcp/…`) |
| Multiple connections per `PeerId` | Stream may attach to wrong mux | Prefer direct for **new** stream when both exist; **close direct only** on asymmetric recovery |
| `open_stream` timeout on relay | Must not nuke relay TCP | `note_stream_open_failure`: reopen on relay if `peer_has_relay_connection` |
| `PeerCondition::NotDialing` | Parallel dial storms | Separate LAN vs circuit app throttles; parallel **links** after connect |

---

### libp2p community lessons (relay v2 — applies directly)

Upstream issues that match Ghal Bol behaviour. When logs disagree with an issue below, fix **code + this doc** — not ad-hoc one-off patches.

| Issue | Symptom in Ghal Bol | Community fix | Our implementation |
|-------|---------------------|---------------|-------------------|
| [rust-libp2p #2513](https://github.com/libp2p/rust-libp2p/discussions/2513) `NoReservation` / circuit DENIED | coord 404; outbound circuit fails; server `circuit DENIED` | **Callee** must `listen_on(/p2p-circuit)` before caller dials | `ensure_wan_relay_circuit` phases B–D; coord presence on `reservation ACCEPTED` |
| [rust-libp2p #2944](https://github.com/libp2p/rust-libp2p/discussions/2944) | Dial fails after reserve | Dial addr must be `…/p2p/<relay>/p2p-circuit/p2p/<dest>` | `coord_lookup` → `dial_dm_peer_addr` with full circuit multiaddr |
| [rust-libp2p #6141](https://github.com/libp2p/rust-libp2p/issues/6141) | `awaiting_relay_circuit` 10–30s while reservation works | `ReservationReqAccepted` may be late/missing — also check `swarm.listeners()` / coord self-lookup | `relay_circuit_listening(swarm)` + coord presence poll |
| [rust-libp2p #6165](https://github.com/libp2p/rust-libp2p/issues/6165) | WAN drops after handover; external addr vanishes | Replacement `listen_on` must not cancel working reservation until new one is active | `relay_reserve_in_flight_ms`; `force` bypasses throttle only — not in-flight; handover uses `clear_wan_listen_state_for_handover` once |
| [rust-libp2p #5741](https://github.com/libp2p/rust-libp2p/discussions/5741) | `dm connection established … (direct)` on inbound relay | Inbound `ConnectionEstablished` `send_back_addr` is `/p2p/<peer>` only — not full circuit path | `InboundCircuitEstablished` → `dm_relay_circuit_pending` before classifying path |
| [PR #4225](https://github.com/libp2p/rust-libp2p/pull/4225) / [#5996](https://github.com/libp2p/rust-libp2p/issues/5996) | `DialPeerConditionFalse`; parallel dials cancel | Default `NotDialing` — one outbound dial slot per `PeerId` | `PeerCondition::NotDialing` + separate LAN/circuit app tracking; parallel **links** after connect, not parallel unconditional dials |
| [#4717](https://github.com/libp2p/rust-libp2p/issues/4717) / [PR #4745](https://github.com/libp2p/rust-libp2p/pull/4745) | Misleading transport errors on circuit fail | Real reason on `OutboundCircuitReqFailed` / `ListenerClosed` | Handle relay listener close + circuit dial errors in swarm handler |
| [#4651](https://github.com/libp2p/rust-libp2p/issues/4651) AutoRelay | Manual reserve after bootstrap HOP + identify | `listen_on` after identify on connected relay | `try_relay_reservation_after_identify`; coord `GET /v1/relay` = static relay list |
| [circuit-v2 spec](https://github.com/libp2p/specs/blob/master/relay/circuit-v2.md) | Presence lost after relay TCP drop | Reservation invalid when bootstrap TCP to relay drops | Re-reserve + re-register; server logs `reservation closed` |
| [PR #5926](https://github.com/libp2p/rust-libp2p/pull/5926) peer-store | Stale identify addrs cause blind wrong dials | Remove addrs on dial failure; don't route from identify alone | Stream-first explicit coord/mDNS addrs; block identify ingest when stream up |
| [#2216](https://github.com/libp2p/rust-libp2p/issues/2216) / [#1135](https://github.com/libp2p/rust-libp2p/issues/1135) | Old LAN ports dialed forever | Identify/external addrs don't expire by default | No peerstore WAN routing; mDNS event-driven LAN only |
| Relay default rate limits [PR #3742](https://github.com/libp2p/rust-libp2p/pull/3742) | `ResourceLimitExceeded`; endless reserve, no chat | Raise `reservation_rate_per_peer` on server | `ghal_bol_server/src/relay.rs` |

**Do not fight upstream:**

- Two concurrent `swarm.dial(same_peer)` without `NotDialing` discipline → use throttled explicit addrs.
- `listen_on` storm while in-flight → cancels reservation (worse than waiting for timeout).
- Inbound relay classified by endpoint path alone → use `InboundCircuitEstablished`.
- DCUtR + identify addrs as primary WAN when coord is configured → disabled; coord + relay only.

Diagnostic log format (grep `category=` / `reason=` / `next=`): implemented in `ghal_bol/src/p2p/connectivity_diag.rs`.

---

## Discovery (Tier 1)

Typical WAN flow ([GHAL_BOL_URI_SCHEME.md](GHAL_BOL_URI_SCHEME.md), [COORDINATION_SERVER.md](COORDINATION_SERVER.md)):

1. Guest scans host QR → stores `public_key_hex`.
2. Both peers register endpoints with `ghal_bol_server`.
3. Lookup `GET /v1/peers/{public_key_hex}` → dial returned endpoints via libp2p.
4. Open `/ghal-bol/msg/1.0.0` stream; speak `ghal_bol_msg_v1`.

**Parallel LAN + WAN on Wi‑Fi:** coord register/lookup on **every configured coord server** + relay circuit (and public TCP when registered) **runs alongside** mDNS/direct TCP when mDNS shows the configured peer on the same network — **both stay active** when connected (§ “Both links active”). On mobile-data/CGNAT without active LAN, WAN only. No Kademlia or public-bootstrap peer discovery when coord paths fail — keep retrying coord.

Coord publishes `tcp`, `quic`, and `libp2p` multiaddrs; `coord_runtime.rs` and `dm_transport/addr.rs` help filter and rank dial targets before libp2p dials.

### Naming — “bootstrap” in logs vs product policy

**Removed:** public IPFS / libp2p bootstrap multiaddrs in `p2p_start` (`bootstrap_peers: []`, `invite_bootstrap=0` in logs).

**Still used internally:** the **coord co-located relay** from `GET /v1/relay` is registered in `bootstrap_peer_ids` and dialed for circuit reservation. Log lines like `bootstrap_dial_error`, `bootstrap_ok`, and `bootstrap redial` refer to **that relay only**, not a bootstrap peer list.

**Do not reintroduce:** Kademlia DHT, IPFS bootnodes, or a static bootstrap peer array for WAN peer discovery. WAN directory = coord HTTP + relay TCP.

### WAN prerequisites (dev and prod — do not regress)

WAN chat between two internet-connected peers requires **both** channels below. ngrok/coord HTTP alone is **not** enough.

| Channel | Dev (`run_server.sh`) | Prod (`coord.ghalbol.com`) |
|---------|----------------------|----------------------------|
| **Coord HTTP** | ngrok `https` → `:8765` | nginx `:443` → `:8765` |
| **Relay TCP** | **bore** → local `:4002` (new remote port **each run**) | public `:4002` on VM (fixed) |

Checklist before blaming the client:

1. `curl …/health` → 200  
2. `curl …/v1/relay | jq` → `"enabled": true`, non-empty `"addrs"`, stable `"peer_id"`  
3. **Relay TCP reachable** — `nc -zv <relay-ip> <port>` from a WAN-like network (must not be `Connection refused`)  
4. Server log: `relay v2 node started` with `advertised=[…]` (not `advertised=[]`)  
5. Server log: `peer registered` after apps unlock (proves `POST /v1/register` succeeded)  
6. Client log: `reservation accepted`, then `coord_registered=true` — not endless `waiting for relay/public listen endpoint before coord register`

**Symptom → cause (coord HTTP logs):**

| What you see | Meaning | Fix |
|--------------|---------|-----|
| `GET /v1/relay` 200, `GET /v1/peers/…` **404 only**, no `peer registered` | Peers never registered — relay circuit or register path failed | Fix relay TCP (bore/firewall); see deploy README |
| `GET /v1/peers/…` 404 after server restart | Stale presence TTL expired; peer not re-registered yet | Restart apps after server+bore so they re-reserve and register |
| `GET /v1/relay` 200 but client `Connection refused` on relay addr | HTTP advertises a **dead** tunnel (bore stopped) | Run `./ghal_bol_server/deploy/run_server.sh`; client refetches live `GET /v1/relay` on next coord tick (no disk cache) |
| One side `reservation accepted`, other side 404 on coord lookup | **Asymmetric CGNAT bug** — phone never registered; Wi‑Fi side looks healthy | Check **phone** log for dial storm + missing `reservation accepted`; see § “CGNAT / mobile-data relay reservation” |
| Phone log: many `coord relay dial` per second, no `bootstrap connection` | Bootstrap **dial storm** — do not add more dials; restore throttle + CGNAT probe path | `issue_bootstrap_dials`, `try_ghalbol_probe_style_circuit_listen` in `retry_stalled_relay_reservations` |
| `relay has no public address advertised` at server start | bore did not run or `GHAL_BOL_RELAY_PUBLIC_*` unset | Use `run_server.sh` (default bore on); read script’s bore-skip reason on stderr |

See [ghal_bol_server/deploy/README.md](../ghal_bol_server/deploy/README.md) § “Regression prevention”.

### LAN vs WAN dial policy

**Route priority when LAN is unknown:** coord lookup + relay circuit (WAN) for every configured contact. **LAN** (mDNS → direct TCP) **adds** when that peer is discovered on the local LAN — not from coord RFC1918 addrs alone. When **both** are available, **both stay active** (§ “Both links active” above) — this section is **not** “use only WAN when coord works.”

- **WAN:** coord lookup + relay circuit (on CGNAT/mobile-data) + public TCP when registered. Lookup tries configured coord servers in order; **stop on first success** for that dial attempt; on reconnect after a drop, repeat the full list. Continues on Wi‑Fi even when mDNS shows the peer on LAN.
- **LAN:** mDNS `Discovered` for the contact → dial direct TCP **in parallel** with WAN — additive, not a replacement for the relay link.
- **Mobile-data:** no blind peer-id dials when coord is configured — explicit relay multiaddrs only.

#### Immediate LAN shift + fast WAN fallback (mDNS-driven)

LAN is faster and stronger, so a contact discovered on the LAN must shift onto it **immediately**, and a contact that **leaves** the LAN must fall back to WAN **without** a long stall — both are explicit connectivity requirements. `dial_mdns_peer` / the mDNS `Expired` handler in `chat_server.rs` enforce this:

- **`Mdns::Discovered`** → `note_peer_on_local_lan` + merge LAN TCP candidates. First connect: one `dial_mdns_lan_addr` per peer when a **new** TCP candidate arrives (event-gated via `try_claim_lan_dial_slot`, not timers). If already connected over relay only, `dial_lan_upgrade` runs on **new** TCP candidate or mux recovery — additive, throttled only by libp2p dial state.
- **`Mdns::Expired`** → `note_peer_mdns_lan_addr_expired` removes **that** multiaddr from `peer_mdns_lan_candidate_addrs` (libp2p often emits `Expired(old_port)` after `Discovered(new_port)` — do not wipe the whole peer on every expire). Only when **no** LAN candidates remain does `forget_peer_on_local_lan` drop the per-peer LAN preference (instead of waiting out `PEER_LAN_SEEN_TTL_MS` = 180s), so dial ranking returns to **WAN-first** at once, and coord/relay lookup re-runs so the peer stays reachable over the internet without delay. If the (now LAN-less) connection later drops, urgent reconnect (§ "Steady connection") takes over.

Do **not** tear down the relay link on LAN discovery (that would drop in-flight messages and deactivate WAN). The upgrade is **additive** — both links active; stream reopen may attach on direct when both links exist.

#### LAN + WAN parallel dial (supersedes “LAN relay vs mDNS race” defer policy — 2026-06-17)

**Current policy:** LAN mDNS TCP and coord relay circuit dials **may run in parallel** for the same peer. Both links may remain connected. Throttles (`should_routed_dial`, `should_circuit_coord_dial`, per-peer intervals) prevent dial storms — **not** mutual deferral.

| Mechanism | Rule |
|-----------|------|
| `dial_mdns_lan_addr` / `dial_dm_peer_addr` | Use `PeerCondition::NotDialing` for additive DM dials — LAN and WAN do not block each other |
| `should_defer_coord_relay_for_lan` | **Removed / always false** — parallel transport supersedes defer-on-LAN |
| `close_dm_relay_connections_for_peer` | **Do not call** on direct connect — keep WAN link active |
| `ConnectionEstablished` relay + direct LAN | **Keep both** — do not `close_connection` on relay because direct is up; log `dm connection established … (relay)` and `(direct)` |
| `coord_dial_from_lookup_addrs` on LAN | May **additive**-dial relay while direct connected — `wan_additive` when already connected; relay-only when direct is up |
| `coord_lookup_dm_peer` | **Do not skip** coord lookup when `peer_on_local_lan` and libp2p-connected — still additive relay dial; skip only when stable mux **and** relay link exists |
| `connect_dm_peer_now` | From `dm_upkeep` when stream is down: **always** `notify_coord_lookup()` when coord configured (even during `circuit_dial_in_flight` — in-flight blocks replacement **dial**, not lookup wake); LAN from mDNS events only |
| `dm_upkeep` coord loop | **Do not skip** lookup merely because `swarm.is_connected` — skip only when stable mux **and** relay link exist (`coord_lookup_upkeep_satisfied`); matches `coord_lookup_dm_peer` |
| `peer_mdns_lan_candidate_addrs` | Live mDNS set only — **not** a dial cache; upkeep does **not** re-dial stale ports |

**Still a regression (unchanged):** `dm_upkeep` re-dialing stale LAN TCP ports from the candidate set; port-ranking heuristics; uncoordinated coord-relay dial spam on CGNAT (many dials/s with no bootstrap TCP); **timer-based failover to next cached mDNS port** (use event-driven `try_mdns_lan_failover_dial` only); **unthrottled public bootstrap redial** when coord is configured (`redial_tick` must use `issue_bootstrap_dials` only).

**Healthy logs:** `mdns dialing` and `coord_lookup_peer ok — dialing` for the same peer on Wi‑Fi is **OK** when throttled; `dm connection established … (direct)` and/or `(relay)`; both `conn=true` briefly during LAN upgrade is OK.

**Application layer (unchanged by parallel transport):** LAN and WAN links are independent; **message and ack state are not**. Every DM frame — text, `ack_received`, `ack_read` — is merged in **`dm_transcript_store` (E)** with monotonic delivery ranks (`read` > `delivered` > `sent`). See [DESIGN.md](DESIGN.md) § “Unified message state (E)”. Do **not** clear WAN links to “simplify” state; do **not** split stores per path.

| Mechanism | Rule |
|-----------|------|
| `dial_mdns_lan_addr` (first connect) | `PeerCondition::NotDialing` — do **not** clear `circuit_dial_in_flight` to “make room” for LAN |
| `dial_dm_peer_addr` (coord / identify) | `PeerCondition::NotDialing` — parallel with in-flight LAN or existing direct link |
| `dial_additive_dm_addr` / `dial_lan_upgrade` | `NotDialing` while relay link exists — additive LAN |
| Duplicate ack on both links | **E** applies higher rank only; `read` wins over `delivered` |

#### LAN relay vs mDNS race (historical — defer policy superseded 2026-06-17)

Older builds **deferred** coord relay while LAN dial was in flight to avoid libp2p oneshot cancel. That caused WAN/LAN split-brain during handover (relay torn down, coord still listing circuit). **Current:** parallel dials + dual active links + unified store — see § “Parallel LAN + WAN transport”.

**Historical broken behaviour:** `should_defer_coord_relay_for_lan` with `connected == true` gate — relay and LAN cancelled each other on first connect.

**Historical log signature (fixed by parallel + NotDialing):** endless `mdns dialing` ~15s apart with competing defer blocking WAN fallback when LAN was dead (stale candidate set) — see § “Ephemeral LAN TCP ports”.

#### Ephemeral LAN TCP ports and stale mDNS candidates (2026-06-15 — read before “fixing” ports)

**There is no stable LAN TCP port to hardcode or guess.** Each `:p2p` / daemon start binds an **ephemeral** TCP port (e.g. Linux `tcp/45787` at 03:01, `tcp/39493` after rebuild at 03:19). That is normal libp2p behaviour. LAN reachability comes from **live mDNS** (`Discovered` / `Expired`), not from remembering a number.

**What actually broke (not “wrong port in config”):** we treated the in-memory mDNS candidate list like a **dial cache** and re-used dead ports after the peer moved.

| Mistake | Effect |
|---------|--------|
| `peer_mdns_lan_candidate_addrs` kept every `Discovered` addr until manually cleared | Old and new ports coexisted (libp2p advertises both briefly before `Expired`) |
| `dm_upkeep` / `connect_dm_peer_now` **re-dialed LAN from that set** every ~16–20s | After native rebuild or listen rebinding, upkeep kept dialing `tcp/45787` while peer listened on `tcp/39493` |
| `should_defer_coord_relay_for_lan` deferred WAN while **any** candidate remained | Stale LAN addrs blocked coord/relay fallback even when LAN was dead |
| “Fixes” that **ranked** ports (highest port, newest timestamp, “preferred” addr, last-in-batch) | Masked the cache bug; agents kept adding heuristics instead of fixing ownership |

**Real log pattern (broken — same Wi‑Fi LAN, two devices):**

```text
# Phone flow snapshot — Linux listen already moved
listen_addrs=…/tcp/38437   dm=[peer:conn=false,stream=false]

# Phone upkeep — stale port from old discovery
03:05:32  mdns dialing …/tcp/45787
03:05:53  mdns dialing …/tcp/45787   ← same dead port, ~20s apart, for minutes
```

Coord and relay were often fine (`coord_registered=true`, `reservation accepted`); chat still stuck at `outbound waiting: not connected` because LAN dials targeted a **stale** RFC1918 port. Probing with `nc` against the old port is misleading — TCP may accept while libp2p/Noise handshake fails, or the port is simply wrong.

**Correct model (current `chat_server.rs`):**

| Path | Owner | Rule |
|------|--------|------|
| **LAN connect** | mDNS events | `Discovered` → dial **that event’s** new LAN TCP (`handle_mdns_discovered_list` + `try_claim_lan_dial_slot`). Dial fail or `Expired` → remove addr; try next set member once (`try_mdns_lan_failover_dial`); then coord. |
| **WAN reconnect** | `dm_upkeep` | `connect_dm_peer_now` → coord lookup + relay dial when stream is down — **may run in parallel** with any in-flight LAN dial from mDNS; **never** re-dial LAN TCP from `peer_mdns_lan_candidate_addrs` on a timer. |
| **Defer relay / defer coord** | — | **`should_defer_coord_relay_for_lan` always false** — superseded 2026-06-17; parallel LAN+WAN. |
| **Candidate set** | `peer_mdns_lan_candidate_addrs` | Live set: add on `Discovered`, remove on `Expired` / dial fail — **not** a ranked dial cache. Order is not meaningful (`peer_mdns_lan_addr` returns any remaining member for failover only). |

**Removed — do not reintroduce:** `rank_mdns_lan_tcp_candidates`, `pick_mdns_lan_tcp_addr`, `peer_mdns_lan_preferred`, highest-port trim, `MDNS_LAN_CANDIDATE_TTL_MS` / port-age ranking, upkeep LAN re-dials, “guess live port” docs or agent workflows.

**Log signatures (fixed — after full app restart with current native):**

```text
03:19:42  mdns discovered …/tcp/39493
03:19:42  mdns discovered …/tcp/44397      ← multiple addrs in one burst is normal
03:19:42  mdns dialing …/tcp/39493         ← once, from discovery (first new LAN TCP)
03:19:44  chat_ready
(silence — no repeated mdns dialing to the same port while conn=true, stream=true)
03:20:13  flow … dm=[peer:conn=true,stream=true] listen_addrs=…/tcp/35407
```

Compare `mdns discovered` / `listen_addrs` in the ~30s `Native/flow` snapshot: the dialed LAN port must match a **current** discovery line, not an addr from minutes ago.

See [DESIGN.md](DESIGN.md) § “Dial strategy — parallel LAN + WAN”. Do not add Dart dial policy, RFC1918 /24 guessing from coord, or port-ranking heuristics (regression: multi-minute LAN stalls).

### LAN stability — cold start and Wi‑Fi toggle (verified 2026-06-16)

**Status:** Short-duration manual testing on Linux desktop + Android shows **cold-start LAN chat** and **Wi‑Fi off/on on the same subnet** both recover without breaking the link. Long soak / LAN↔WAN↔LAN cycles depend on § **Network truth** and § **Asymmetric LAN↔WAN mux recovery** (2026-06-19).

**Network mode:** See § **Network truth — OS default route** (canonical). Summary: OS default transport + validated flag drive `profile=`; `if_addrs` is secondary; `Native/flow` logs `os=wifi|cell/validated/…`.

**What broke Wi‑Fi switch (not “wrong port in config”):**

| Bug | Symptom | Fix |
|-----|---------|-----|
| **Full kick same tick as circuit dial** (2026-06-19) | Stream drop → full LAN handover + WAN circuit dial same upkeep tick → `NoReservation` / relay churn | **Defer** close/rebind ephemeral TCP while `circuit_dial_in_flight`; soft nudge + `pending_full_lan_kick_reason`; flush full kick when dial completes — § “Deferred full LAN kick”. **Not** soft-only forever. |
| **Soft mDNS-only upkeep** (no pending full kick) | Repeating `LAN soft rediscovery`, never `fresh ephemeral TCP listen` | Upkeep must call full `kick_lan_dm_rediscovery_after_handover` when no circuit dial in flight, or flush pending queue |
| **Recovery throttle double-consume** | Upkeep called `should_run_lan_recovery` then only soft-restarted mDNS; full kick was throttled out | Let `kick_lan` own the throttle; do not pre-consume it before a soft restart |
| **Daemon restart / empty `peers_on_local_lan`** | Link down, no `mdns discovered` after sync | `peer_eligible_for_lan_handover`: prior mDNS/LAN sighting or Wi‑Fi pending outbox with LAN history — not bare foreground room for WAN-only peers; not every roster peer |
| **Linux missing link-up event** | Same-subnet toggle: profile stayed `lan`, no handover key change, no kick | `linux_network::poll_wifi_link_up_transition` → `notify_network_change` → forced kick |
| **Poll path skipped DM-down-on-LAN** | Streams down on LAN but 1s poll never kicked | `dm_down_on_lan = on_lan && needs_lan` (not only on connectivity notify) |
| **Profile lag after mobile↔Wi‑Fi toggle** | `profile=` wrong for minutes; ticks/outbox stuck | § **Network truth** — `os=` must flip in ~1s; rebuild native if missing |
| **Asymmetric mux loop** | `reopen peer off LAN` every 1s; relay churn; one side `(direct)` other `(relay)` | § **Asymmetric LAN↔WAN mux recovery**; `close direct … relay kept` |
| **Full kick on every dial fail** (earlier) | `closed stale LAN ephemeral TCP listener` every ~200ms | Failover removes addr + `notify_dm_presence_wake`; full kick only on handover / upkeep / connectivity |

**Required recovery sequence after Wi‑Fi toggle** (`kick_lan_dm_rediscovery_after_handover`):

1. Purge `peer_mdns_lan_candidate_addrs` for DM peers; clear `lan_dial_in_flight` / `lan_candidates_exhausted`
2. `ensure_lan_tcp_listen(handover=true)` — close stale ephemeral listeners, bind fresh `/ip4/0.0.0.0/tcp/0`
3. `restart_mdns_behaviour(force=true)`
4. `notify_stream_reopen`, `clear_coord_lookup_backoff_all`, `schedule_register_presence_force`

**Triggers:** Android/Linux connectivity notify; `lan_handover_upkeep` when link down + no candidate (5s throttle); `handle_lan_interface_drift` (`lan`→`lan` key change); mobile-data→LAN `handle_lan_path_restored`.

#### Deferred full LAN kick (2026-06-19 — parallel LAN + WAN)

**Problem observed:** A transient LAN/stream drop in the same `dm_upkeep` tick as an outbound **relay circuit dial** could run full `kick_lan_dm_rediscovery_after_handover` (close ephemeral TCP listeners, rebind `/tcp/0`, purge mDNS candidates) while libp2p was still handshaking on the WAN path. Symptom: relay `NoReservation`, coord 404 flap, minutes to recover — even when coord HTTP was healthy.

**Why full kick matters (unchanged):** Wi‑Fi toggle / cold start LAN recovery **requires** fresh ephemeral TCP + force mDNS. Soft mDNS **alone** without a follow-up full kick leaves stale ports (regression: never `mdns discovered`).

**Policy — two phases, not either/or:**

| Phase | When | What runs |
|-------|------|-----------|
| **Interim (soft)** | LAN upkeep needs recovery **and** `any_dm_circuit_dial_in_flight` | `soft_lan_rediscovery_nudge` — mDNS restart (no force), stream reopen, presence wake. **Queue** `pending_full_lan_kick_reason`. WAN circuit dial continues undisturbed. |
| **Full kick** | No circuit dial in flight **or** pending queue flush after dial ends | `kick_lan_dm_rediscovery_after_handover` — purge candidates, `ensure_lan_tcp_listen(handover=true)`, force mDNS, coord backoff clear. |

**Implementation (`chat_server.rs`):**

- `lan_handover_upkeep_if_needed`: circuit in flight → `note_pending_full_lan_kick` + soft; else → full kick immediately.
- `try_flush_pending_full_lan_kick`: called at start of LAN upkeep (after `expire_stale_circuit_dials` in `dm_upkeep`) when in-flight window clears.
- Connectivity notify / forced `kick_lan(..., force=true)` still run full kick and clear pending.

**Healthy logs (deferred path):**

```text
LAN soft rediscovery — circuit dial in flight — defer fresh TCP listen
… circuit dial completes or times out …
LAN DM rediscovery — deferred full kick (link down, no mDNS candidate yet)
LAN handover — fresh ephemeral TCP listen for mDNS
mdns discovered …/tcp/XXXXX
```

**Do not regress:**

- Using soft path when **no** circuit dial is in flight (Wi‑Fi toggle must get full kick immediately).
- Never flushing pending (soft forever — same as old soft-only bug).
- Full kick on every LAN dial `OutgoingConnectionError` (failover + presence wake only; full kick on handover/upkeep).

**Success logs (within ~5–15s after Wi‑Fi back):**

```text
LAN DM rediscovery — Wi‑Fi back (connectivity notify)
  or LAN DM rediscovery — link down, no mDNS candidate yet
LAN handover — fresh ephemeral TCP listen for mDNS
mdns restarted after LAN handover
mdns discovered …/tcp/XXXXX
mdns dialing …/tcp/XXXXX          ← same port as discovered
dm connection established … (direct)
chat_ready
```

**Regression signatures — do not ship:**

```text
LAN soft rediscovery … (repeating, no deferred full kick / fresh ephemeral TCP listen)
LAN upkeep — nudge mDNS (link down, no candidate yet)   ← old soft-only label (removed)
closed stale LAN ephemeral TCP listener (handover) every ~200ms
coord dialing … relay circuit while on LAN with no mdns dialing first
```

**Future agents — do not reintroduce:** soft mDNS **without** pending full kick when upkeep would otherwise run `kick_lan`; pre-throttle before `kick_lan`; port ranking; upkeep LAN re-dial from candidate cache; full `kick_lan` on every LAN dial `OutgoingConnectionError`; Flutter `p2p_notify_network_change`.

### Event-driven async — avoid assumed timers (canonical)

**Product rule (general — not limited to dial or handover):**

Whenever **policy** needs an outcome whose **duration is unknown** (connect, listen, reserve, lookup, stream open, register, path shift, …), **do not** drive that policy on guessed intervals (`sleep(N)`, grace windows, “retry every tick until maybe ready”). Instead:

1. **Worker (B)** — owns the long-running or async operation until the stack reports a **fact** (success, failure, disconnect, new addr, HTTP response, …).
2. **Policy (A)** — **subscribes** to those facts and reacts **immediately** (open stream, drain outbox, failover, invalidate state, shift LAN↔WAN).
3. **Timers** — only where the **stack or flood prevention** requires them (TCP/circuit in-flight observation, storm throttles, keepalive below idle timeout, register dedupe when endpoints unchanged). Never as a substitute for “we don't know when B will finish.”

The **A / B subscriber model** is an **analogy** for this split — one example is “A needs a peer connected; B keeps dialing until libp2p notifies.” The **same pattern** applies anywhere Rust/product waits on work it cannot time-bound.

**Where this applies in Ghal Bol (non-exhaustive):**

| Area | A (policy — react on signal) | B (worker — unknown duration) | B → A signals (examples) |
|------|------------------------------|-------------------------------|---------------------------|
| **LAN / WAN connect** | Stream-first connect, parallel route pick (LAN + WAN throttled) | `swarm.dial`, mDNS browse, relay reserve | `ConnectionEstablished`, `OutgoingConnectionError`, mDNS `Discovered`/`Expired` |
| **Network handover** | `kick_lan` once, purge stale addrs, reopen streams — **parallel** with WAN recovery | mDNS restart, ephemeral listen, relay reserve | Connectivity notify, profile change, relay `ListenerClosed` |
| **Coord / WAN backup** | Lookup when stream down, outbox waiting, or LAN path exhausted | HTTP lookup, relay circuit dial | Lookup ok/404/error, bootstrap connected, reservation accepted |
| **DM stream** | Open mux, drain outbox, read-ack gate | `open_stream`, mux read/write | Stream ready, `receiver is gone`, connection closed |
| **Presence / register** | Publish when endpoints **change** | `POST /v1/register`, relay listen set | Endpoint diff, reservation accepted, handover kick |
| **Flutter UI** | Render transcript/ticks from native stores | — | Poll is **display only** — never connect/ack policy |

**Anti-pattern (any area):** A polls or sleeps because B might be done “by now”; tick loops that re-kick the same recovery (mDNS, stream reopen, coord) without a new event; tuning `N` seconds instead of wiring the subscriber.

**Allowed timers (guardrails only — all areas):**

- In-flight observation while B runs (`LAN_DIAL_IN_FLIGHT_MS`, `CIRCUIT_DIAL_IN_FLIGHT_MS`) — track B, do not replace its events
- Storm throttles (`should_issue_bootstrap_dial`, `should_routed_dial`, `should_throttle_register`)
- Keepalive ping < idle connection timeout
- Backoff after **confirmed** failure (404, refused) — not preemptive “wait before trying”

**Forbidden (regressions — often handover, same rule everywhere):** grace windows blocking coord; **`dm_upkeep` skipping coord lookup when libp2p-connected**; tick-polled recovery without a new event; shortening timer constants as a “speed fix”; timer-driven re-dial from stale caches; **45s `LAN_HANDOVER_GRACE_MS` stream-reopen window** (removed — stream reopen is event-driven via `ConnectionEstablished`, mDNS `Discovered`, `notify_stream_reopen` on handover kicks, and `upkeep_dm_peers` when connected).

#### Connectivity — one application of the rule (`chat_server.rs`)

| Event (B finished or failed) | A reacts immediately |
|------------------------------|-------------------|
| Android `ConnectivityManager` / Linux `wl*` operstate up / profile change | `kick_lan_dm_rediscovery_after_handover` **once** (fresh listen + force mDNS + purge stale addrs) |
| mDNS `Discovered` (direct LAN TCP) | `dial_mdns_lan_addr` / `dial_lan_upgrade` on **that** addr |
| mDNS `Expired` / LAN dial `OutgoingConnectionError` | Drop addr, failover candidate or `notify_coord_lookup` |
| `ConnectionEstablished` (DM) | `note_connection_path`, clear in-flight dials, open chat stream |
| `ConnectionClosed` / full DM disconnect | `recover_dm_peer_after_disconnect` (stream reopen + mDNS or coord) — **not** full `kick_lan` (avoids killing a link that just connected) |
| LAN dial no longer in flight + candidates exhausted | `notify_coord_lookup` (WAN backup) |

**`dm_upkeep` (~1s)** drains outbox, read-ack retries, and work **already queued by events** — it is **not** the connect owner and must **not** poll “is handover still active?” to re-kick mDNS, reopen streams, or pause all coord on a clock.

### Roaming

- **This device** — Android `ConnectivityManager` callbacks in `:p2p` (thin hook → Rust `android_network.rs`), Linux `wl*` operstate poll (`linux_network.rs` on `network_tick`), 1s interface profile poll, WAN relay recovery when coord URL is set. **Flutter must not** call network-change RPCs; UI only polls for display.
- **Wi‑Fi return (soft handover)** — when `has_active_lan` flips false→true (e.g. mobile-data → Wi‑Fi while rmnet/CGNAT iface still visible), `handle_lan_path_restored` runs: ephemeral LAN TCP listen, mDNS behaviour restart (throttled), clear `lan_candidates_exhausted`, `mark_dm_reconnect_urgent_unless_live_direct_stream`, coord register refresh — **no** `coord_invalidate` / forced WAN recovery. On-LAN DHCP drift (`lan`→`lan` handover key change) uses `handle_lan_interface_drift` → full `kick_lan_dm_rediscovery_after_handover`. Leaving LAN **immediately** purges mDNS state then full WAN handover.
- **Wi‑Fi toggle (same subnet, both still on LAN)** — see § **“LAN stability — cold start and Wi‑Fi toggle”** (canonical). Summary: OS link-up hint → `notify_network_change` → **`kick_lan_dm_rediscovery_after_handover` once**; upkeep repeats full kick (throttled) when link down + no mDNS candidate — **not** soft mDNS-only restart. mDNS **`handle_mdns_discovered_list`** dials LAN TCP on every `Discovered` event when disconnected. **LAN connect is mDNS event-driven only** — no upkeep LAN re-dial from cache.
- **Coord tick** — periodic lookup (~5s) plus immediate lookup when send is queued and peer is not connected.

### Steady connection when both peers are online (do not regress)

The link between two online contacts must stay **steady** — no idle drops, and fast recovery from a transient blip — so messages are not delayed by a full reconnect. Mechanisms in `chat_server.rs` enforce this:

1. **Keepalive ping** — `ChatBehaviour.ping` pings every `PING_INTERVAL_SECS` (8s), comfortably under `SWARM_IDLE_CONNECTION_TIMEOUT_SECS` (45s Android / 300s desktop). A healthy-but-quiet chat connection is therefore never dropped between messages. Do **not** remove ping or raise the interval above the idle timeout. **Idle open DM stream** (hub closed, no inbound frames) is **not** stale — `dm_peer_stream_up` / `dm_link_needs_recovery` must not churn coord/LAN while the mux writer is live.
2. **Partial connection close** — libp2p may hold several parallel TCP paths to the same DM peer (brief mDNS burst before first connect). When one path closes, emit `PeerDisconnected` / clear the stream writer / `note_disconnected` **only if** `!swarm.is_connected(peer)` — otherwise log at debug and keep the live stream. **LAN dials are mDNS event-driven** (`handle_mdns_discovered_list`); **`dm_upkeep` → `connect_dm_peer_now`** may trigger coord/WAN **and** dial a live mDNS addr when present — **in parallel**, throttled — but must **not** re-dial stale LAN TCP from `peer_mdns_lan_candidate_addrs` on a timer (§ “Ephemeral LAN TCP ports”). Do **not** open parallel mDNS dials while a LAN dial is already in flight (`lan_dial_in_flight` → skip). On `OutgoingConnectionError` for a LAN TCP addr, remove that addr and fail over to the next set member once (`try_mdns_lan_failover_dial`), then `notify_coord_lookup` when exhausted. **Linux desktop** idle link timeout is **120s** (not 300s) so quiet LAN links recycle sooner after listen-port changes — still above keepalive ping interval.
3. **Urgent reconnect** — on full `dm peer disconnected` (no libp2p link left), the peer’s key enters a bounded urgent window (`DM_RECONNECT_URGENT_WINDOW_MS`, 30s) via `mark_dm_reconnect_urgent`. While urgent (`is_pk_reconnect_urgent`), coord lookup **skips** the `peer_not_on_server` 404 backoff and the 1s upkeep tick retries reconnect immediately, instead of waiting for the 5s coord tick or the exponential backoff. The window is cleared on successful reconnect. **Relay bootstrap loss** must not mark urgent / coord-lookup peers that already have a **live direct LAN stream** (`mark_dm_reconnect_urgent_unless_live_direct_stream`).
4. **Reserve on all configured coord relays in parallel, throttled per relay** — `try_relay_reservations` issues `listen_on(/p2p-circuit)` to every connected **Ghal Bol relay** (from `GET /v1/relay` on each configured coord URL) that is not already circuit-listening, and `try_relay_reservation` enforces a per-relay throttle (`RELAY_RESERVE_THROTTLE_MS`). Do **not** use public IPFS bootstrap peers for relay reservation or peer discovery. The client **dials relay base TCP first**, then reserves after identify. The anti-pattern is re-issuing `listen_on` **every tick** (a 1s storm), **not** covering all relays once: serializing onto a single relay let one pending-but-never-accepted reservation block the others, so WAN readiness took minutes or never came up. Per-relay throttling keeps the parallel fan-out storm-free.
5. **Bootstrap relay dial throttle (CGNAT)** — `issue_bootstrap_dials` / `should_issue_bootstrap_dial` limit redundant `swarm.dial` to the same coord relay (10s normal, 3s minimum during forced WAN recovery). Uncoordinated dials from `maybe_refresh_ghalbol_relay`, `ensure_coord_relays_connected`, and `redial_tick` **without** this throttle have repeatedly caused a **dial storm** that prevents bootstrap TCP from ever completing on mobile-data/CGNAT.
6. **Stream mux recovery** — on `open_stream` failure (`receiver is gone`) or send `chat stream closed`, `request_dm_stream_reopen` clears the writer and reopens on the **existing** libp2p connection on the next upkeep tick. Do **not** `disconnect_peer_id` while a direct route may still work. Outbox retries use the same stream — no teardown on `on_wire` timeout alone.
7. **Presence wake (inactive → active)** — when **this** device re-announces on coord (`try_register_presence` ok, relay `reservation accepted`, app `ui_visible=true`, or network handover), `notify_dm_presence_wake` runs on the next `dm_upkeep` tick (~1s): clears `peer_not_on_coord` backoff for known contacts **without** a live stream and opens a 30s urgent reconnect window. Peers with `dm_peer_stream_up` are skipped. Coord is the WAN phone book; mDNS is the LAN fast path — both are discovery inputs only (stream-first).

### CGNAT / mobile-data relay reservation (recurring regression — read before changing relay code)

This bug has come back **multiple times**. It is **not** the same as “relay server down” or “Linux relay OK so WAN is fine”. Symptom pattern is often **asymmetric**:

| Side | Typical profile | What you see |
|------|-----------------|--------------|
| Wi‑Fi / LAN desktop | `profile=lan` | `bootstrap connection` → `reservation accepted` → `coord_registered=true` in ~5–10s |
| Mobile-data / CGNAT phone | `profile=mobile-data`, `cgnat=true` | Endless `CGNAT listen addr only — waiting for libp2p relay circuit`; **no** `bootstrap connection`, **no** `reservation accepted` |
| Wi‑Fi side looking up phone | — | Coord lookup **404 forever** for the phone’s public key (phone never registered) |
| Phone looking up Wi‑Fi side | — | `coord_lookup_peer ok` + dials peer’s relay circuit — **one-way** visibility, still **no chat** |

### Outbound peer relay dials vs own reservation (do not conflate)

Two different relay-related actions must stay separate in `dial_dm_peer_addr` / coord lookup paths:

| Action | Target | Requires own `relay_circuit_listening`? | Throttle |
|--------|--------|----------------------------------------|----------|
| **Bootstrap / coord relay TCP** | `GET /v1/relay` base multiaddr | No (establishes path to infrastructure) | `issue_bootstrap_dials` / `should_issue_bootstrap_dial` |
| **Own circuit reservation** | `listen_on(…/p2p-circuit)` on coord relay | Yes — result is **your** publishable WAN addr | `try_relay_reservation`, CGNAT probe `listen_on` |
| **Outbound dial to peer** | Peer’s `/p2p-circuit` addr from coord lookup | **No** — peer already registered; client routes through coord relay bootstrap TCP | `should_routed_dial` in `dial_dm_peer_addr` (2s per peer for coord tag) |

**Why:** A CGNAT phone can reach a Wi‑Fi desktop as soon as coord lookup returns the desktop’s relay circuit. Waiting for the phone’s own `reservation accepted` before dialing the peer adds tens of seconds of dead WAN for no transport reason.

**Regression (2026-06-10 — reverted):** A brief gate blocked relay peer dials when `wan_discovery_via_coord_only()` + mobile coord strategy + `!relay_circuit_listening(swarm)`. Logs showed `coord_lookup_peer ok — dialing 1 addr(s)` immediately followed by `skip relay dial …: self relay circuit not ready yet` for 30–40s; the registered desktop saw endless coord **404** for the phone. **Do not reintroduce** — use per-peer `should_routed_dial` only; reserve/bootstrap throttles address dial **storms**, not legitimate first connect to an already-registered peer.

**Log signatures (phone / CGNAT side):**

- Many `coord relay dial …` lines **per second** (not once every 10–12s) — **bootstrap dial storm**
- `node_ready` fires but relay never comes up
- Never `reserving circuit on …` or `ghalbol circuit listen (probe path) …` after the first seconds
- Never `WAN not ready at startup — begin recovery pass` on builds **before** the fix (recovery started too late)

**Root causes (both must be guarded in code):**

1. **Dial storm** — `GET /v1/relay` refetch (every ~5s when not registered), WAN recovery, and bootstrap redial all called `swarm.dial` to the same relay with **no per-relay throttle**. Pending dials pile up; bootstrap TCP never completes on cellular/CGNAT even when the relay is reachable from Wi‑Fi.
2. **Wrong path while bootstrap TCP is pending** — `try_relay_reservations` only runs after `any_bootstrap_connected`. On CGNAT, bootstrap TCP can stay pending for a long time if dials are spammed. The fix is **probe-style** `listen_on(…/p2p-circuit)` via `try_ghalbol_probe_style_circuit_listen` when `on_mobile_data_path()` and not yet circuit-listening — same idea as `examples/relay_probe.rs`. libp2p’s relay client establishes the link through `listen_on`, not only through a completed outbound dial + `ConnectionEstablished`.

**Required behaviour (`chat_server.rs` — do not regress):**

| Mechanism | When |
|-----------|------|
| `issue_bootstrap_dials` + `should_issue_bootstrap_dial` | Every coord-relay `swarm.dial`; clears on network handover |
| Probe-style `listen_on` at **startup** | `coord_only` + `on_mobile_data_path()` + no circuit yet, right after first `dial_coord_relays` |
| `begin_wan_recovery` at **startup** | Same condition — do not wait for the first `coord_tick` on CGNAT |
| Probe in `retry_stalled_relay_reservations` | `!any_bootstrap_connected` + `on_mobile_data_path()` + not circuit-listening |
| `try_relay_reservation` after identify | Normal path once bootstrap TCP **is** connected (Wi‑Fi / fast paths) |
| `should_routed_dial` in `dial_dm_peer_addr` | Every coord/mDNS peer addr dial — prevents oneshot cancel / relay rate-limit storms **without** blocking first connect |
| `circuit_dial_in_flight_ms` | Blocks replacement coord relay dials for **45s** after each outbound circuit dial (`circuit_dial_in_flight_blocks`); cleared on `ConnectionEstablished` / `OutgoingConnectionError`; `expire_stale_circuit_dials` on dm upkeep after 45s — **do not** clear early while libp2p is still handshaking (oneshot cancel) |

**Do not “fix” this by:**

- Removing probe-style listen from CGNAT paths — Wi‑Fi-only testing will still pass while phones stay broken.
- Calling probe-style `listen_on` on **every** `coord_tick` / `try_relay_reservations` when bootstrap is still dialing — that poisons `RELAY_RESERVE_THROTTLE_MS` (see anti-pattern § “Steady connection” item 3). Probe belongs at startup and in `retry_stalled` when bootstrap is **not** connected, plus after identify when connected.
- Assuming one device’s `reservation accepted` means chat works — **both** peers must register on coord.
- **Blocking outbound peer relay dials until own circuit listens** — confuses “dial peer from coord” with “reserve own circuit”; see § “Outbound peer relay dials vs own reservation” above.

**Verify on two devices:** Android on mobile data + Linux on Wi‑Fi. Within ~15s of `:p2p` start, **phone** log must show `reservation accepted` (or probe path then accepted), then `coord_registered=true`. Until then, the other side’s coord lookup 404 for that peer is **expected**, not a coord-server bug.

---

## Multiple coord / relay servers

The app accepts a **list** of coord server base URLs via **`GHAL_BOL_COORD_URLS`** in `ghal_bol_ui/env/.env.development` / `.env.production` (JSON array or comma-separated; no hardcoded URLs in Rust). Today a single entry is typical (`https://coord.ghalbol.com`); the API is an array for future redundancy. Each entry is a full **coord + relay** pair — HTTP presence plus a co-located Circuit Relay v2 node (`GET /v1/relay` on that host).

| Action | Policy |
|--------|--------|
| **Register** | Register presence (and relay circuit addr) on **every** reachable coord in the list |
| **Lookup** | When dialling a peer, try coord servers in order; **stop on first successful lookup + connect** |
| **Reconnect** | After a connection drop while internet is active, repeat lookup across the full list |
| **Coord unreachable** | Keep retrying all entries on the regular interval; **LAN (mDNS) unaffected** |

Do not substitute Kademlia DHT or public libp2p bootstrap peers when a coord lookup fails — WAN discovery requires coord/relay.

---

## libp2p relay-client WAN state machine (client)

All WAN circuit reservation must go through **`ensure_wan_relay_circuit`** in `chat_server.rs` — not ad-hoc `listen_on` from scattered ticks. rust-libp2p relay-client behaviour that agents must respect:

| Constraint | Why |
|------------|-----|
| HOP pins to **one** bootstrap TCP link | Dual-stack happy-eyeballs can open v4+v6; prune to one anchor before `listen_on`. **`listen_on` must use the live HOP TCP multiaddr** from `bootstrap_tcp_conns`, not the dial-cache addr (desktop IPv6-unreachable + IPv4 HOP was a common stall). |
| New `listen_on(/p2p-circuit)` **cancels** in-flight reservation | Never re-issue while `relay_reserve_in_flight_ms` is set (30s timeout). |
| **Identify** on bootstrap before reserve | Prefer `bootstrap_identified` after `Identify::Received`; if Identify was drained during `bootstrap_publishable_listen`, allow `listen_on` after `RELAY_TCP_HOP_FALLBACK_MS` (~800ms) on an established bootstrap TCP link. |
| Startup listen wait | `bootstrap_publishable_listen` forwards **all** swarm events through `handle_swarm_event` — never drop Identify/Relay in a partial match. |
| Probe `listen_on` **only** when bootstrap TCP is down | CGNAT path; never parallel with active bootstrap dials. |
| Throttle redundant dials / listens | `issue_bootstrap_dials`, `RELAY_RESERVE_THROTTLE_MS` — storms break mobile CGNAT. |

Phases: dial bootstrap (all families, one throttle window) → Identify → prune HOP → settle 450ms → **one** `listen_on` → `ReservationReqAccepted` → coord register.

---

When neither peer is directly reachable (home‑NAT desktop ⇄ CGNAT phone), WAN needs a **Ghal Bol relay** that reliably grants Circuit Relay v2 reservations. `ghal_bol_server` runs its **own** relay node next to each HTTP coordinator. The HTTP API stays a lightweight presence phone book; the relay only carries brief NAT‑traversal traffic until **DCUtR** upgrades the client pair to a direct connection. **Public IPFS bootstrap peers are not used** for peer discovery or relay reservation.

**Server (`ghal_bol_server/src/relay.rs`)**
- Circuit Relay v2 + Identify (`/ghal-bol/1.0.0`) + Ping over **TCP + Noise + Yamux** (libp2p 0.56, protocol‑identical to the client).
- Stable ed25519 identity persisted at `<data_dir>/relay_ed25519.key` → constant PeerId across restarts. (The relay's **own** node key is ed25519 — that is fine; it is infrastructure, not a user identity.)
- **The relay's `libp2p` MUST enable the `secp256k1` feature** (`ghal_bol_server/Cargo.toml`). Ghal Bol **clients authenticate with their secp256k1 device identity** (golden rule 7 / [IDENTITY.md](IDENTITY.md)). The Noise handshake authenticates the remote's identity public key, so a relay built **without** `secp256k1` cannot decode/verify a secp256k1 client and **drops the connection mid‑handshake** — the client sees `Decode(Io(UnexpectedEof))`, the circuit listener closes (`addrs=[]`), `coord_registered=false`, and **no real device can ever reserve a circuit** (every device uses a secp256k1 key). A minimal probe using an ed25519 key will *appear* to work and hide this — always test the relay with a **secp256k1** key (`PROBE_SECP256K1=1` in `examples/relay_probe.rs`).
- **Dual-stack (IPv4 + IPv6, IPv6 preferred).** The relay listens on the configured address **and** the counterpart-family wildcard on the same port (`GHAL_BOL_RELAY_LISTEN` default `0.0.0.0:4002` ⇒ also `[::]:4002`), so it accepts both IPv4 and IPv6 clients. A counterpart-listen failure (host without that stack) logs a warning and continues single-stack.
- Env: `GHAL_BOL_RELAY_ENABLE` (default on), `GHAL_BOL_RELAY_LISTEN` (default `0.0.0.0:4002`), `GHAL_BOL_RELAY_PUBLIC_HOST` (→ advertises **both** `/dns6/<host>/tcp/<port>` and `/dns4/<host>/tcp/<port>`, IPv6 first) or `GHAL_BOL_RELAY_PUBLIC_ADDRS` (comma‑separated multiaddrs). **The relay TCP port must be open to the internet**; advertise the public host or clients cannot reserve. For native IPv6 reachability the host needs an `AAAA` record; on IPv4‑only/NAT64 carriers the `/dns*` host is mapped to a routable address client-side regardless.
- **Relay rate limiters (production).** libp2p `relay::Config::default()` installs per-peer/per-IP rate limiters (~**one circuit per 2 minutes** per source peer). Ghal Bol’s `:p2p` node retries DM reconnect every ~2 s when the outbox has pending rows (background — **not** gated on opening a chat room). If the coord relay still uses those default limiters, the server logs `relay circuit DENIED … ResourceLimitExceeded` while clients log endless `coord_lookup_peer ok — dialing …/p2p-circuit` with no `dm peer connected`. `ghal_bol_server/src/relay.rs` clears `reservation_rate_limiters` and `circuit_src_rate_limiters` and raises pool caps instead. **Redeploy the server binary** after changing relay config; client-only rebuilds cannot fix this.
- `GET /v1/relay` → `{ enabled, peer_id, addrs }` (addrs are dialable bases without `/p2p/<id>`; both `/dns6` and `/dns4` are returned, IPv6 first).
- **Registration circuit expansion (`routes.rs`).** On `POST /v1/register`, `/dns*/…/p2p-circuit` endpoints are duplicated with resolved `/ip6/…` **and** `/ip4/…` aliases (IPv6 first) so TCP-only clients (Android has no libp2p DNS transport) can dial a peer's relay circuit by concrete IP over whichever family routes.

**Client (`ghal_bol`)**
- At swarm startup, for **each** configured coord URL, `coord_runtime::fetch_all_ghalbol_relays` fetches live `GET /v1/relay` (no on-disk cache) and `network_transport::resolve_relay_bootnodes` resolves dialable bases into **both IP families** — `/ip4/<public>/tcp/<port>/p2p/<id>` **and `/ip6/<routable>/tcp/<port>/p2p/<id>`** (IPv6 sorted first; product policy is "IPv6 preferred when it works"). This is required for IPv6‑only / NAT64 mobile carriers: there the OS resolver (DNS64) synthesizes an IPv6 address for the relay's `/dns4` hostname and the literal IPv4 base has no route — keeping only IPv4 (the old behaviour) left such devices unable to reserve a circuit and therefore unreachable. `is_trusted_bootstrap_dial_addr` accepts a public IPv4 **or** a globally routable IPv6 (incl. NAT64 `64:ff9b::/96`). `issue_bootstrap_dials` dials **all** resolved families for a relay within one throttle window (happy‑eyeballs) so a preferred‑but‑unroutable family never starves the other. libp2p's relay client pins HOP to **one** bootstrap TCP link per relay, so `prune_duplicate_relay_bootstrap_connections` closes extras and keeps the best family (`relay_bootstrap_family_rank` — IPv6 on global‑v6 LAN, IPv4 on CGNAT/mobile when both connect). **Circuit reservation** is then a single `listen_on(…/p2p-circuit)` on that anchor only (`relay_circuit_listen_addr`). Do **not** issue multi‑family `listen_on` while two bootstrap TCP links are still up — HOP and circuit addr must match. The client **dials base TCP** (throttled), prunes to one link, then after identify requests the circuit. Probe-style `listen_on` runs only from `retry_stalled_relay_reservations` when bootstrap TCP is still **not** connected (not in parallel with active bootstrap dials). The resulting `/p2p-circuit` is registered in coord presence; recovery retries in § "Steady connection". **If the advertised relay TCP port is unreachable** (dev: dead bore/ngrok tunnel; prod: firewall), clients log `relay TCP unreachable`, clear in-memory relay state, refetch `GET /v1/relay` on the next refresh tick, and never register on coord until the tunnel is fixed.

### Caching policy (canonical)

**Rule:** cache (especially on disk) **only** when the data is **immutable for practical purposes** — user identity, contacts, message history, preferences. If the value **can change** and code that **relies on a cached copy could break the app** (chat, WAN, LAN connect), **do not cache it** — use live sources. Exception only with an explicit product decision recorded in this section.

**Live sources for transport (never disk):**

| Data | Source |
|------|--------|
| Relay bootstrap addr/port | `GET /v1/relay` each start + `maybe_refresh_ghalbol_relay` |
| Peer WAN dial addrs | `GET /v1/peers/{public_key}` on upkeep / urgent reconnect |
| LAN TCP port | mDNS `Discovered` / `Expired` events only |

**OK on disk (immutable / user-owned):**

| Data | Why |
|------|-----|
| Encrypted keystore | User secret until rotation |
| `contacts_v1.json` | User roster; `public_key_hex` is identity anchor, not a dial addr |
| `chat_transcript_v1.json` | User message history |
| Preferences / aliases | UI state |

**In-memory only (this `:p2p` run):** storm throttles (`should_routed_dial`, `lan_dial_in_flight`, reserve throttle); `ghalbol_relay_state` from last successful `GET /v1/relay` (**cleared** on relay TCP failure); mDNS candidate set `peer_mdns_lan_candidate_addrs` (add on `Discovered`, remove on `Expired`/dial-fail — **upkeep must not re-dial from it**).

**Historical note:** older docs mentioned relay disk cache for boot. **This section is canonical** — `ghalbol_relay.json` was removed; legacy files are deleted on start.

**Removed / forbidden (relying on these broke P2P):**

| Item | Why |
|------|-----|
| `ghalbol_relay.json` | Bore port changes every dev server run |
| `coord_cached_dial_addrs` | Stale addrs raced live mDNS |
| Upkeep LAN re-dial from candidate set | Ephemeral ports change every restart |
| Port-ranking heuristics | Masked stale-cache bugs |
| Dart-side dial/lookup caches | Routing lives in Rust only |

**Before adding any cache**, answer: (1) Is it immutable user data? (2) If stale for ~30s, does connectivity or chat break? If (2) yes → no cache. (3) If you still need it, document the special reason here.
- **Network handovers** (wifi ⇄ mobile ⇄ different LAN): relay re-reservation rides `handle_network_path_change` → `retry_stalled_relay_reservations`, so the circuit is re-reserved and re-registered on the new path without a libp2p restart.

---

## Helper modules (not a separate transport)

| Path | Role |
|------|------|
| `ghal_bol/src/p2p/chat_server.rs` | libp2p swarm, streams, outbox, ack policy |
| `ghal_bol/src/p2p/network_transport.rs` | `LocalNetworkProfile`, `OsNetworkSnapshot`, OS merge, relay resolution (no Kademlia) |
| `ghal_bol/src/android_network.rs` | Android `ConnectivityManager` JNI probe (`:p2p` only) |
| `ghal_bol/src/linux_network.rs` | Linux default route + Wi‑Fi operstate |
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
| **One bootstrap TCP per coord relay** on the best IP family — libp2p relay HOP uses `directly_connected_peers.first()` only | `prune_duplicate_relay_bootstrap_connections` |
| Bootstrap dials may race v4+v6; reservation `listen_on` is `…/p2p-circuit` on the **kept** anchor only | `relay_circuit_listen_addr` + `relay_reservation_circuit_addrs` |
| Bootstrap relay **dials** throttled per relay (no dial storm on CGNAT) | `issue_bootstrap_dials` / `should_issue_bootstrap_dial` |
| CGNAT/mobile: probe-style relay reservation when bootstrap TCP pending | `try_ghalbol_probe_style_circuit_listen` at startup + `retry_stalled_relay_reservations` |
| Outbound peer relay dials after coord lookup are **not** gated on own `relay_circuit_listening` | `dial_dm_peer_addr` + `should_routed_dial` only |
| LAN discovery upgrades a relay-only link (additive — both links active); LAN loss drops LAN pref + WAN already connected | `dial_lan_upgrade` / `note_peer_mdns_lan_addr_expired` / `forget_peer_on_local_lan` |
| LAN dials primarily from mDNS events; upkeep may dial live mDNS addr + coord **in parallel** — never re-dial stale cached LAN ports | `handle_mdns_discovered_list`; `connect_dm_peer_now` |
| **OS default route** drives `profile=lan` vs `mobile-data` (not `if_addrs` alone) | `refresh_os_network_truth`, `merge_os_network_truth`, `android_network.rs`, `linux_network.rs` |
| Asymmetric handover: close **direct** `ConnectionId`s only when relay exists; one DM mux | `close_direct_dm_connections`, `reconcile_stale_lan_mux_for_wan` |

---

## AI handoff — common mistakes

1. **Reimplementing ack policy in Dart** — forbidden; see DESIGN.md.
2. **Assuming libp2p was removed** — it was not; read this file.
3. **Reintroducing gossipsub** for 1:1 DM — wrong model.
4. **Requiring mutual QR** — guest-only host key from QR is intentional.
5. **Clearing `pending_read_acks` on leave** — breaks DESIGN leave backlog.
6. **Restarting libp2p on every contact upsert** — use hot `register_dm_peer` instead.
7. **Kademlia / public-bootstrap WAN discovery when coord is down** — forbidden; WAN requires coord/relay. LAN (mDNS) still works.
8. **Slow WAN fallback after LAN loss** — mDNS `Expired` must re-kick coord/relay lookup immediately; do not wait on LAN TTL.
9. **Skipping relay TCP dial for the coord relay** — client must dial `GET /v1/relay` base addr (throttled), then reserve; on CGNAT also use probe-style `listen_on` when bootstrap is not connected yet (§ “CGNAT / mobile-data relay reservation”).
10. **Treating coord HTTP OK as WAN OK** — `GET /v1/relay` 200 with unreachable relay TCP → endless `GET /v1/peers/…` 404; fix bore/firewall first.
11. **Reintroducing static `bootstrap_peers` for WAN** — `bootstrap_peers: []` is intentional; only coord relay from `/v1/relay` is the WAN dial target.
12. **Uncoordinated bootstrap relay dial spam** — refetch + WAN recovery + redial calling `swarm.dial` every 1–2s without `should_issue_bootstrap_dial` prevents bootstrap TCP from completing on mobile-data; log shows many `coord relay dial` lines, never `bootstrap connection`.
13. **Blind `swarm.dial(peer_id)` for DM peers** — `libp2p_peer_dial_pending` used dial-as-probe and started peerstore multi-dials to `::1`, stale LAN, bare `/p2p/` instead of coord `/p2p-circuit`; symptom: `Failed to negotiate transport protocol(s)` with many bad addrs, LAN ok, WAN dead. Explicit coord/mDNS multiaddrs only.
13. **Removing CGNAT probe reservation** — `try_ghalbol_probe_style_circuit_listen` at startup / when `!any_bootstrap_connected` is required for phones; Wi‑Fi-only tests hide the regression.
14. **One-sided relay OK** — Linux `reservation accepted` while Android stuck on `CGNAT listen addr only` means chat will not work; both peers must register on coord.
15. **Blocking peer relay dials until own circuit listens** — `skip relay dial … self relay circuit not ready yet` after `coord_lookup_peer ok` stalls WAN 30–40s on CGNAT; peer outbound dials only need coord relay bootstrap TCP + peer registered. See § “Outbound peer relay dials vs own reservation”.
16. **Blocking LAN upkeep during WAN recovery** — `lan_handover_upkeep` returning early when `wan_recovery_active && !relay_circuit_listening`, or `relay_lost_on_lan` re-kicking full handover every 5s while coord/ngrok is down — LAN never gets stable mDNS. **Fix:** parallel upkeep (WAN reserve + LAN listen/mDNS); `relay_lost_on_lan` false when `wan_recovery_active`. See § “Parallel LAN + WAN transport”.
17. **Racing coord relay dials against mDNS LAN on Wi‑Fi** — **superseded 2026-06-17:** parallel `mdns dialing` + `coord dialing` on Wi‑Fi is **OK** when throttled. Regressions are **uncoordinated dial spam** and **stale LAN port re-dial from upkeep** — see § “LAN relay vs mDNS race” (historical).
18. **Caching transport or dial targets** — coord lookup addr cache, frozen mDNS LAN addr, on-disk relay cache, upkeep re-dials from `peer_mdns_lan_candidate_addrs`, Dart routing cache. **Canonical rule:** TRANSPORT.md § “Caching policy” — disk only for immutable user data; if staleness could break connectivity, fetch live.
19. **Port guessing / ranking heuristics** — highest-port-wins, “preferred” mDNS addr, TTL-based pick, or `nc` probes instead of reading mDNS lifecycle + `Native/flow` listen_addrs. See § “Ephemeral LAN TCP ports”.
19. **Soft mDNS-only Wi‑Fi switch recovery** — `restart_mdns_behaviour` without `ensure_lan_tcp_listen(handover=true)` and candidate purge; or pre-consuming `should_run_lan_recovery` then skipping full `kick_lan`. Symptom: endless `LAN upkeep — nudge mDNS`, no `mdns discovered`. See § “LAN stability — cold start and Wi‑Fi toggle”.
20. **`if_addrs`-only network mode** — `profile=lan|mobile-data` from interface scan without `getActiveNetwork` / default route — minutes wrong after toggle. See § **Network truth**.
21. **Asymmetric mux reset loop** — `reopen peer off LAN` every 1s + `disconnect_peer_id` on relay while `dm_direct_conn_ids` stale — ticks/outbox into void. See § **Asymmetric LAN↔WAN mux recovery**.

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

Canonical LAN + Wi‑Fi toggle behaviour: § **“LAN stability — cold start and Wi‑Fi toggle”** and § **Roaming**. Rows below are history; intermediate fixes may be superseded.

| Date | Change |
|------|--------|
| 2026-06-19 | **Network truth (OS default route):** `OsNetworkSnapshot` from Android `getActiveNetwork` + `NET_CAPABILITY_VALIDATED` and Linux `/proc/net/route` + `wl*` operstate; `profile=lan|mobile-data` no longer driven by lagging `if_addrs` alone; `Native/flow` logs `os=wifi|cell/validated/…`. § **Network truth**. |
| 2026-06-19 | **Asymmetric LAN↔WAN mux recovery:** `dm_direct_conn_ids` + `close_direct_dm_connections` (keep relay); `reconcile_stale_lan_mux_for_wan` throttled; link reset reopens on relay instead of full disconnect. Fixes `reopen peer off LAN` storms and tick/outbox into void. § **Asymmetric LAN↔WAN mux recovery**. |
| 2026-06-19 | **WAN blind peerstore dial (root cause):** `libp2p_peer_dial_pending` called `swarm.dial(peer_id)` as a probe — started multi-dials to `::1`/stale LAN/bare `/p2p/` instead of coord `/p2p-circuit`; removed; coord mode disables `try_routed_dial_impl`. § anti-regression #13. | removed `swarm.is_connected` / `dm_peer_stream_up`-only skips — additive relay while direct up via `coord_lookup_upkeep_satisfied`; removed 45s `LAN_HANDOVER_GRACE_MS` timer path (stream reopen is event-driven); `connect_dm_peer_now` always wakes coord lookup when configured. § “Event-driven async”, § “Both links active”. |
| 2026-06-19 | **Coord lookup on LAN-connected peers:** `coord_lookup_dm_peer` no longer returns early when `peer_on_local_lan` — additive relay dial while direct is up; stable-mux skip only when relay link already exists. § “Both links active”, § “LAN + WAN parallel dial”. |
| 2026-06-18 | **Parallel LAN+WAN doc + code alignment:** LAN upkeep no longer blocked by `wan_recovery_active`; `relay_lost_on_lan` does not re-kick handover while WAN recovery is stuck (coord/ngrok down). Supersedes changelog rows that said “defer LAN upkeep during WAN recovery”. § “Parallel LAN + WAN transport”, § “Hybrid coord presence”. |
| 2026-06-18 | **Caching policy (canonical):** disk only for immutable user-owned data; transport reachability live-only (`ghalbol_relay.json` removed). § “Caching policy”. |
| 2026-06-18 | **Removed on-disk relay cache (`ghalbol_relay.json`):** relay bootstrap coords come from live `GET /v1/relay` only; legacy files deleted on `:p2p` start; in-memory state cleared on relay TCP failure. |
| 2026-06-18 | **LAN cold start + WAN relay hardening (partial — see parallel row above):** `peer_eligible_for_lan_handover` gates `lan_rediscovery_peer_set` (foreground room, Wi‑Fi outbox, or prior mDNS — not ghost/WAN-only roster). ~~`lan_handover_upkeep` defers while `wan_recovery_active`~~ **superseded** — parallel upkeep. Coord transport lookup uses fixed 3s throttle (not 404 curve). Bootstrap: prune spare HOP before reserve, never mid-reserve; IPv4-only when v4 advertised (LAN/mobile/CGNAT). `maybe_refresh_ghalbol_relay` stops on circuit listen (not `coord_registered` alone). |
| 2026-06-18 | **WAN + LAN + switching verified (dev):** hybrid coord presence stable on Linux desktop + Android over ngrok + bore. Fixes: never register relay bootstrap TCP as client endpoint; prune stale published LAN ports; ~~defer LAN upkeep during WAN recovery~~ **superseded** — parallel; relay renewal vs re-reserve race; `prefer_direct_dm_path_over_relay` only on active LAN; coord lookup error bodies. § “Hybrid coord presence”, § “LAN ↔ WAN handover”. |
| 2026-06-18 | **Relay reservation cancel loop (root WAN flap):** clean `ListenerClosed` during libp2p renewal was calling `kick_relay` → new `listen_on` every ~2s, cancelling the live reservation. Track `ReservationReqAccepted` time; skip re-reserve during renewal window; skip `notify_dm_presence_wake` on renewals; do not call `ensure_wan_relay_circuit` when circuit already listening. Throttle coord re-register when HTTP transport down. |
| 2026-06-16 | **LAN stability (verified short test):** cold-start LAN and Wi‑Fi off/on on same subnet recover on Linux + Android. Fixes: `linux_network.rs` (sysfs operstate → `notify_network_change`); `platform_wifi_linked`; full `kick_lan` on connectivity notify and in `lan_handover_upkeep` (not soft mDNS-only); `dm_down_on_lan` on 1s poll when streams down. § “LAN stability — cold start and Wi‑Fi toggle”. Removed port ranking / upkeep LAN re-dial (earlier). |
| 2026-06-16 | **Event-driven async (general rule):** § “Event-driven async — avoid assumed timers”; A/B subscriber model for connect, handover, lookup, reserve, stream, register. AGENTS + DESIGN.md aligned. |
| 2026-06-16 | **Architecture:** Android Wi‑Fi probe in Rust (`android_network.rs`); `:p2p` Kotlin registers callbacks only; removed Flutter `p2p_notify_network_change`. |
| 2026-06-17 | **Parallel LAN + WAN transport:** LAN and WAN stacks always run together; per-peer direct + relay links both stay **active** (not torn down when the other succeeds); `should_defer_coord_relay_for_lan` superseded; unified message state (E) in Rust with monotonic delivery merge. § “Parallel LAN + WAN transport”, DESIGN.md § “Unified message state (E)”. |
| 2026-06-17 | **WAN coordination:** relay server owns `/p2p-circuit` on coord; client `coord_registered` only after self-lookup + local circuit listen; `NoReservation` → urgent re-lookup. § “WAN coordination”. |
| 2026-06-16 | **LAN connect model (supersedes port-ranking experiments):** mDNS event-driven dial primary path; `connect_dm_peer_now` also triggers coord/WAN in parallel when stream is down (no stale LAN re-dial from cache). ~~45s handover grace defers coord~~ **superseded 2026-06-17** — parallel LAN+WAN. § “Ephemeral LAN TCP ports”, § “LAN relay vs mDNS race”. |
| 2026-06-15 | **Ephemeral LAN ports / stale mDNS cache:** documented ephemeral TCP + candidate-set lifecycle; stopped upkeep re-dials to stale ports. § “Ephemeral LAN TCP ports”. |
| 2026-06-15 | **Wi‑Fi return handover:** `has_rfc1918_on_wifi` + `lan_restored`; soft handover on mobile-data→LAN; immediate mDNS purge on `lan → mobile-data`. |
| 2026-06-15 | **FORBIDDEN hub session patch (reverted):** `lastApplySucceeded` / per-frame session retry — broke P2P. Never reintroduce. DESIGN.md § “FORBIDDEN — reverted 2026-06-15”. |
| 2026-06-14 | **Stream-first symmetric connect:** canonical connect model from the original serverless build (seconds to connect, one stream per contact, single upkeep owner). Coord/relay/mDNS are **parallel discovery inputs** — one mux per contact; avoid **uncoordinated dial spam**, not parallel LAN+WAN. § “Stream-first symmetric connect”; DESIGN.md § same. Removed incorrect “coord relay first on outbox” dial guidance. |
| 2026-06-13 | **LAN relay vs mDNS race (regression fix):** on Wi‑Fi, defer coord relay dials while a **direct LAN dial is in flight** — **before** first connect. Removed coord lookup addr cache; mDNS uses live candidate list per peer. § “LAN relay vs mDNS race”, § “Caching policy (P2P)”. |
| 2026-06-11 | **Bootstrap TCP prune (libp2p relay HOP pin):** happy-eyeballs left **two** coord-relay TCP links (v6 then v4); libp2p's relay client sends all HOP (reserve + routed DM dial) on `directly_connected_peers[relay].first()` only. When v6 connected first on mobile-data but could not carry HOP, v4 bootstrap was ignored — server saw `client connected` ×2, no `reservation`/`circuit` events. `prune_duplicate_relay_bootstrap_connections` keeps one link (IPv4 on mobile-data path, IPv6 when LAN has global v6); reservation uses that anchor only. |
| 2026-06-11 | **Relay reservation:** dual-family bootstrap **dials** kept; `prune_duplicate_relay_bootstrap_connections` keeps one TCP on best family (`relay_bootstrap_family_rank`); one `listen_on(…/p2p-circuit)` on that anchor. No startup probe while bootstrap dials run; `relay_reservation_active` gates on accepted reservation only. |
| 2026-06-11 | **Relay reservation regression fix:** with `bootstrap_relay_addr` set, `relay_reservation_circuit_addrs` returned the base TCP addr without `/p2p-circuit`, so `listen_on` failed (`relay reserve listen …` empty error). Fixed via `relay_circuit_listen_addr`. |
| 2026-06-11 | Dual-family bootstrap **dial** retained (`issue_bootstrap_dials`); dual `listen_on` per family removed once anchor exists — HOP is single-connection. |
| 2026-06-10 | **Outbound peer relay dials (regression fix):** removed gate that skipped coord peer relay dials when own `relay_circuit_listening` was false on CGNAT — caused `skip relay dial … self relay circuit not ready yet` for ~40s while lookup succeeded. Peer circuit dials use `should_routed_dial` only; own reservation stays on bootstrap/probe/reserve path. § “Outbound peer relay dials vs own reservation”. |
| 2026-06-10 | **WAN startup latency:** dial coord relay + CGNAT probe before `bootstrap_publishable_listen`; process `ConnectionEstablished` during listen wait (bootstrap TCP was completing while events were ignored → ~minute relay delay on mobile). Shorter mobile listen wait; faster CGNAT probe throttle; probe `listen_on` Err clears retry state; cap coord 404 backoff for DM contacts at 3s. |
| 2026-06-09 | **CGNAT relay reservation regression (recurring):** § “CGNAT / mobile-data relay reservation” — asymmetric symptom (Wi‑Fi OK, phone 404), bootstrap **dial storm** vs missing probe `listen_on`, required fixes (`issue_bootstrap_dials`, startup WAN recovery, `retry_stalled` probe path). Steady-connection item 4 + invariants + AI handoff items 12–14. |
| 2026-06-09 | **WAN dev regression docs + relay dial path:** § “Naming — bootstrap in logs”, § “WAN prerequisites”, symptom→cause table. Client dials coord relay TCP before reservation; clears `ghalbol_relay.json` on refused dial. Dev bore port changes each `run_server.sh` run — document refetch requirement. |
| 2026-06-09 | **Coord-required WAN, no KAD/bootstrap discovery:** WAN peer discovery requires configured coord/relay servers; Kademlia DHT and public libp2p bootstrap peers are not fallbacks. LAN (mDNS) still works when coord is down. Added § "Multiple coord / relay servers". |
| 2026-06-06 | **Immediate LAN shift + fast WAN fallback:** mDNS `Discovered` now upgrades a relay-only link to a direct LAN connection (`dial_lan_upgrade`, `PeerCondition::NotDialing`, per-peer throttle; additive, never tears down WAN); per-connection direct/relay tracking added (`peers_direct_conns`). mDNS `Expired` drops the LAN preference immediately (no 180s TTL wait) and re-kicks WAN discovery. See § "Immediate LAN shift + fast WAN fallback". |
| 2026-06-06 | **Relay secp256k1 fix (root cause of WAN failure):** `ghal_bol_server`'s `libp2p` was missing the `secp256k1` feature, so the relay dropped every secp256k1 client (i.e. every real device) mid Noise handshake (`Decode(UnexpectedEof)` → circuit listener `addrs=[]` → `coord_registered=false`). Added `secp256k1` (+`ed25519`) to the server. `examples/relay_probe.rs` gained `PROBE_SECP256K1=1` because the ed25519-only probe masked the bug. Removed debug-only scaffolding committed during the hunt (relay-reservation repro test module, `keycheck` example). |
| 2026-06-06 | **Ghal Bol relay co-located with coord** (`ghal_bol_server/src/relay.rs`, `GET /v1/relay`): clients reserve a circuit on our own reliably-granting relay (preferred over public IPFS bootstraps) and register that `/p2p-circuit` in presence — fixes WAN for NAT⇄CGNAT pairs where neither side is directly reachable. |
| 2026-06-06 | WAN regression fix: relay reservations fan out to **all** eligible bootstraps again (per-relay throttle), reverting the one-at-a-time scheme that serialized behind a stuck pending reservation and stalled WAN for minutes. |
| 2026-06-05 | Steady-connection hardening: keepalive `ping`, urgent DM reconnect (no 404 backoff), one-at-a-time relay reservation. |
| 2026-05-31 | Canonical transport doc; libp2p confirmed as production stack. |
