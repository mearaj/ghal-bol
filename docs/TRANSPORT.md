# Transport — libp2p data plane

**Status:** **libp2p is the production P2P transport.** A prior plan to replace libp2p with a custom native QUIC/TCP stack was **evaluated and discarded** (May 2026). This document is the canonical reference for how peers connect today.

**For AI / new sessions:** Read [AGENTS.md](../AGENTS.md) and [DESIGN.md](DESIGN.md) first. Transport changes must **not** move ack policy, outbox, or transcript merge into Flutter. **Connectivity policy:** [STORY.md](STORY.md) § **`# Story` onward** overrides conflicting guidance here — **human-owned; agents read only, never edit or `git checkout` STORY.md**. The opening sections (`Current issues`, `# Now`, `# Next`) are human backlog, **not** implementation spec; do not throttle relay/WAN recovery from them (see AGENTS.md § “STORY.md — do not misread the first sections”).

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

## Stream-first symmetric connect (connect layer)

**Canonical model** — documented in [DESIGN.md](DESIGN.md) § “Stream-first symmetric connect”. Ghal Bol: one live DM stream per contact, ~1s `dm_upkeep`, stream reopen on mux failure without tearing down the libp2p link. Coord + relay + mDNS supply dial addrs (WAN/LAN lookup); they do not each spawn independent dials.

```text
Per contact (every ~1s dm_upkeep):
  if live DM stream writer (dm_peer_stream_up):
    noop — no coord lookup, no disconnect, no identify dial
  else if not libp2p-connected:
    pick ONE route (LAN mDNS OR coord relay — never both in parallel)
    swarm.dial once (throttled)
    open /ghal-bol/msg/1.0.0 OR accept inbound on same handler
  else if connected but no stream:
    open_stream once on existing connection
  if stream up && outbox pending → drain
```

| Principle | Implementation |
|-----------|------------------|
| Both listen | Swarm listens; inbound streams accepted on `/ghal-bol/msg/1.0.0` |
| One stream per contact | Stream writer map keyed by peer / `public_key_hex`; `dm_peer_stream_up` → upkeep noop |
| Symmetric | Outbound `open_stream` and inbound accept use the same handler — no fixed caller/listener role |
| Send = connect | `send_text_dm` / outbox retry share the stream-first path; hub room open not required |
| Single dial owner | `dm_upkeep` owns per-peer connect; coord lookup + mDNS **only when stream is down** |

**Discovery vs connect:** coord `GET /v1/peers`, relay circuit multiaddrs, and mDNS are **inputs** to the single dial decision. They must not each spawn independent concurrent dials to the same peer (libp2p cancels one — “oneshot canceled” — and connect stalls for ~15s loops).

**Latency target:** seconds to `peer_connected` + `chat_ready` when the remote has finished WAN registration (phases A–D in § “WAN prerequisites”) — matching the original build’s feel.

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

## End-to-end WAN phases (both peers — read before changing transport)

WAN chat is a **pipeline with hard gates**. Each device must complete phases A→D before the **other** device can dial it. Opening a chat room or sending a new message does **not** substitute for these gates; `:p2p` drives them from `p2p_start`, outbox restore, `dm_upkeep`, and `coord_tick`.

```text
Per device (Android :p2p / Linux daemon)
──────────────────────────────────────
A. Swarm up          p2p_start, dm_peers registered
B. Relay bootstrap   TCP to GET /v1/relay peer (log: bootstrap connection)
C. Own reservation   listen_on(…/p2p-circuit) after Identify on HOP
                     (log: reservation accepted, relay listen addr)
D. Coord presence    Client POST /v1/register with public + optional LAN tcp only;
                     relay server upserts `/p2p-circuit` on reservation (identify `;pk=`).
                     CGNAT-only clients poll GET /v1/peers/self until circuit visible.
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
| `issue=no_dial_addrs \| reason=no dial addrs — coord has no record` | E blocked by remote D | Same as 404 — do not treat as “dial broken” |
| `coord_lookup_peer ok — dialing` but no `dm peer connected` | F | Circuit dial failing — check server `circuit ACCEPTED/DENIED`; also check stream-first violations (parallel dials cancelling each other) |
| `mdns dialing` then `coord dialing` within ~2s (same peer, Wi‑Fi) | F | **Stream-first violation** — relay raced LAN in-flight; see § “LAN relay vs mDNS race” |
| `mdns dialing` only for 15s+ then relay (no parallel race) | F (LAN path) | LAN TCP failing (firewall/wrong port); relay should follow after in-flight window — expected sequence |
| `issue=… ResourceLimitExceeded` | F | Relay server rate limiters — redeploy `ghal_bol_server` |
| `dm peer connected` + `outbox resync` | G ✓ | Pending transcript outbox drains — no new user send required |

### libp2p community lessons (relay v2 — applies directly)

| Issue | Lesson for Ghal Bol |
|-------|---------------------|
| [rust-libp2p #2513](https://github.com/libp2p/rust-libp2p/discussions/2513) `NoReservation` / circuit DENIED | **Callee must** `listen_on(/p2p-circuit)` on the relay before caller’s circuit dial works. 404 on coord lookup means callee never reached phase D. |
| [rust-libp2p #2944](https://github.com/libp2p/rust-libp2p/discussions/2944) | After `ReservationReqAccepted`, connection still needs correct dial multiaddr format: `…/p2p/<relay>/p2p-circuit/p2p/<dest>`. |
| Reservation valid only while bootstrap TCP up | [circuit-v2 spec](https://github.com/libp2p/specs/blob/master/relay/circuit-v2.md): disconnect from relay invalidates reservation — server logs `reservation closed`. Client must re-reserve + re-register. |
| `listen_on` while in-flight cancels prior reservation | Never re-issue faster than `RELAY_RESERVE_THROTTLE_MS`; one HOP anchor only. |
| Relay server default rate limiters | ~1 circuit / 2 min / peer — incompatible with 2s reconnect upkeep; clear in `ghal_bol_server/src/relay.rs`. |

Diagnostic log format (grep `category=` / `reason=` / `next=`): implemented in `ghal_bol/src/p2p/connectivity_diag.rs`.

---

## Discovery (Tier 1)

Typical WAN flow ([GHAL_BOL_URI_SCHEME.md](GHAL_BOL_URI_SCHEME.md), [COORDINATION_SERVER.md](COORDINATION_SERVER.md)):

1. Guest scans host QR → stores `public_key_hex`.
2. Both peers register endpoints with `ghal_bol_server`.
3. Lookup `GET /v1/peers/{public_key_hex}` → dial returned endpoints via libp2p.
4. Open `/ghal-bol/msg/1.0.0` stream; speak `ghal_bol_msg_v1`.

**WAN first:** coord register/lookup on **every configured coord server** + relay circuit (and public TCP when registered). **LAN only** when mDNS shows the configured peer on the same network (direct TCP). No Kademlia or public-bootstrap peer discovery when coord paths fail — keep retrying coord.

Coord publishes `tcp`, `quic`, and `libp2p` multiaddrs; `coord_runtime.rs` and `dm_transport/addr.rs` help filter and rank dial targets before libp2p dials.

### Naming — “bootstrap” in logs vs product policy

**Removed:** public IPFS / libp2p bootstrap multiaddrs in `p2p_start` (`bootstrap_peers: []`, `invite_bootstrap=0` in logs).

**Still used internally:** the **coord co-located relay** from `GET /v1/relay` is registered in `bootstrap_peer_ids` and dialed for circuit reservation. Log lines like `bootstrap_dial_error`, `bootstrap_ok`, and `bootstrap redial` refer to **that relay only**, not a bootstrap peer list.

**Do not reintroduce:** Kademlia DHT, IPFS bootnodes, or a static bootstrap peer array for WAN peer discovery. WAN directory = coord HTTP + relay TCP ([STORY.md](STORY.md)).

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
| `GET /v1/peers/…` 404 after server restart | Stale presence TTL expired; peer not re-registered yet | Restart apps after server+bore; clear `ghalbol_relay.json` if bore port changed |
| `GET /v1/relay` 200 but client `Connection refused` on relay addr | HTTP advertises a **dead** tunnel (bore stopped, wrong port cached) | Run `./ghal_bol_server/deploy/run_server.sh`; refetch `/v1/relay`; client clears cache on refused dial |
| One side `reservation accepted`, other side 404 on coord lookup | **Asymmetric CGNAT bug** — phone never registered; Wi‑Fi side looks healthy | Check **phone** log for dial storm + missing `reservation accepted`; see § “CGNAT / mobile-data relay reservation” |
| Phone log: many `coord relay dial` per second, no `bootstrap connection` | Bootstrap **dial storm** — do not add more dials; restore throttle + CGNAT probe path | `issue_bootstrap_dials`, `try_ghalbol_probe_style_circuit_listen` in `retry_stalled_relay_reservations` |
| `relay has no public address advertised` at server start | bore did not run or `GHAL_BOL_RELAY_PUBLIC_*` unset | Use `run_server.sh` (default bore on); read script’s bore-skip reason on stderr |

See [ghal_bol_server/deploy/README.md](../ghal_bol_server/deploy/README.md) § “Regression prevention”.

### LAN vs WAN dial policy

**WAN first** for every configured contact. **LAN** (mDNS → direct TCP) **only** when that peer is discovered on the local LAN — not from coord RFC1918 addrs alone.

- **WAN:** coord lookup + relay circuit (on CGNAT/mobile-data) + public TCP when registered. Lookup tries configured coord servers in order; **stop on first success** for that dial attempt; on reconnect after a drop, repeat the full list.
- **LAN exception:** mDNS `Discovered` for the contact → prefer direct TCP for that peer.
- **Mobile-data:** no blind peer-id dials when coord is configured — explicit relay multiaddrs only.

#### Immediate LAN shift + fast WAN fallback (mDNS-driven)

LAN is faster and stronger, so a contact discovered on the LAN must shift onto it **immediately**, and a contact that **leaves** the LAN must fall back to WAN **without** a long stall — both are explicit STORY requirements. `dial_mdns_peer` / the mDNS `Expired` handler in `chat_server.rs` enforce this:

- **`Mdns::Discovered`** → `note_peer_on_local_lan` + merge LAN TCP candidates. First connect: one `dial_mdns_lan_addr` per peer when a **new** TCP candidate arrives (event-gated via `try_claim_lan_dial_slot`, not timers). If already connected over relay only, `dial_lan_upgrade` runs on **new** TCP candidate or mux recovery — additive, throttled only by libp2p dial state.
- **`Mdns::Expired`** → `note_peer_mdns_lan_addr_expired` removes **that** multiaddr from `peer_mdns_lan_candidate_addrs` (libp2p often emits `Expired(old_port)` after `Discovered(new_port)` — do not wipe the whole peer on every expire). Only when **no** LAN candidates remain does `forget_peer_on_local_lan` drop the per-peer LAN preference (instead of waiting out `PEER_LAN_SEEN_TTL_MS` = 180s), so dial ranking returns to **WAN-first** at once, and coord/relay lookup re-runs so the peer stays reachable over the internet without delay. If the (now LAN-less) connection later drops, urgent reconnect (§ "Steady connection") takes over.

Do **not** tear down the existing connection on LAN discovery (that would drop in-flight messages); the upgrade is additive and the stream follows the better path on reopen.

#### LAN relay vs mDNS race (2026-06-13 regression — do not reintroduce)

On Wi‑Fi/LAN, **mDNS direct TCP and coord relay circuit dials must not race** for the same peer before the first `ConnectionEstablished`. libp2p cancels one dial (“oneshot canceled”); the survivor is often the slower relay path while the LAN dial stalls ~15s (`lan_dial_in_flight`), then repeats forever — chat never connects on LAN even though mDNS shows the correct RFC1918 addr.

**Broken behaviour (regression):** `should_defer_coord_relay_for_lan` required `connected == true` before deferring relay dials. On **first connect**, coord lookup returned relay addrs immediately after mDNS `Discovered`, so relay and mDNS LAN TCP dials ran in parallel.

**Required behaviour (`chat_server.rs`):**

| Mechanism | Rule |
|-----------|------|
| `rank_mdns_lan_tcp_candidates` / `pick_best_mdns_lan_tcp_addr` | **Removed** — do not rank LAN addrs by port or timestamp. Dial the addr from the current mDNS `Discovered` event; failover tries the next set member after dial fail. |
| `dial_mdns_lan_addr` | Skip while `circuit_dial_in_flight` — never cancel an in-flight relay-circuit dial with `PeerCondition::Always` LAN TCP. |
| `should_defer_coord_relay_for_lan` | On active Wi‑Fi/LAN, defer coord **relay** dials only while a **direct** LAN TCP dial is in flight (`lan_dial_in_flight`) or a **direct** link is open. **Do not** defer because mDNS addrs remain in the set — stale membership must not block WAN. |
| `connect_dm_peer_now` | **Coord/WAN only** from `dm_upkeep`. LAN dials are **mDNS event-driven** (`handle_mdns_discovered_list`), not upkeep retries to a cached port. |
| `dial_dm_peer_addr` | Block relay-circuit dials while `lan_dial_in_flight` on Wi‑Fi/LAN (same race as above). |
| `handle_mdns_discovered_list` | Merge addrs from the event; on **new** LAN TCP for a disconnected DM peer, dial **that event's addr** (not a ranked pick from cache). LAN upgrade while on relay uses the new addr from the same event. |
| `coord_dial_from_lookup_addrs` | Strip coord-published RFC1918 addrs on LAN (stale ports); call `should_defer_coord_relay_for_lan` and drop relay addrs from the ranked list when deferring. |
| `peer_mdns_lan_candidate_addrs` | Live set from mDNS `Discovered` minus `Expired`/dial-fail removals — **not** a dial cache with port heuristics. Upkeep does **not** re-dial from this set. |

**Log signatures (broken LAN):**

- `mdns dialing … /ip4/192.168.x.x/tcp/…` then within ~2s `coord_lookup_peer ok — dialing … via relay circuit` for the **same** peer on Wi‑Fi
- No `dm connection established` / `peer_connected` for minutes; mDNS retries ~15s apart
- Sends stuck: `outbound blocked: coord lookup (additive)`

**Log signatures (fixed LAN):** `mdns dialing` without a competing relay dial **while LAN dial is in flight**; then `dm connection established … (direct)` when LAN works, or `LAN candidates exhausted` + `coord_lookup_peer ok` when WAN is needed.

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
| **WAN reconnect** | `dm_upkeep` | `connect_dm_peer_now` → **coord lookup only** — **never** re-dial LAN from memory on a timer. |
| **Defer relay** | in-flight / direct only | `should_defer_coord_relay_for_lan` while LAN dial is in flight or direct link is up — **not** because stale addrs sit in the HashMap. |
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

See [DESIGN.md](DESIGN.md) § “Dial strategy — WAN first”. Do not add Dart dial policy, RFC1918 /24 guessing from coord, or port-ranking heuristics (regression: multi-minute LAN stalls).

### LAN stability — cold start and Wi‑Fi toggle (verified 2026-06-16)

**Status:** Short-duration manual testing on Linux desktop + Android shows **cold-start LAN chat** and **Wi‑Fi off/on on the same subnet** both recover without breaking the link. This is the most stable LAN handover configuration tested to date. Long soak / multi-hour idle is not yet covered here.

**Network state (no extra library required):** `if_addrs` alone lags after Wi‑Fi toggle. Authoritative hints:

| Platform | Source | Module |
|----------|--------|--------|
| Android | `ConnectivityManager` (`TRANSPORT_WIFI`) | `android_network.rs` |
| Linux | `/sys/class/net/wl*/operstate` down→up | `linux_network.rs` |
| Both | 1s profile poll + libp2p connection/stream state | `network_tick`, `chat_server.rs` |

Do **not** add a cross-platform “network library” unless a new platform lacks these hooks. libp2p does not report OS link-up; recovery is triggered by OS hints + P2P disconnect events.

**What broke Wi‑Fi switch (not “wrong port in config”):**

| Bug | Symptom | Fix |
|-----|---------|-----|
| **Soft mDNS-only upkeep** | Repeating `LAN upkeep — nudge mDNS`, never `mdns discovered` | Upkeep calls full `kick_lan_dm_rediscovery_after_handover` (fresh listen + force mDNS + purge), same as cold start |
| **Recovery throttle double-consume** | Upkeep called `should_run_lan_recovery` then only soft-restarted mDNS; full kick was throttled out | Let `kick_lan` own the throttle; do not pre-consume it before a soft restart |
| **Linux missing link-up event** | Same-subnet toggle: profile stayed `lan`, no handover key change, no kick | `linux_network::poll_wifi_link_up_transition` → `notify_network_change` → forced kick |
| **Poll path skipped DM-down-on-LAN** | Streams down on LAN but 1s poll never kicked | `dm_down_on_lan = on_lan && needs_lan` (not only on connectivity notify) |
| **Port ranking / upkeep LAN re-dial** (earlier) | Same stale `mdns dialing …/tcp/PORT` every ~20s | Removed; LAN connect is mDNS event-driven only (§ “Ephemeral LAN TCP ports”) |
| **Full kick on every dial fail** (earlier) | `closed stale LAN ephemeral TCP listener` every ~200ms | Failover removes addr + `notify_dm_presence_wake`; full kick only on handover / upkeep / connectivity |

**Required recovery sequence after Wi‑Fi toggle** (`kick_lan_dm_rediscovery_after_handover`):

1. Purge `peer_mdns_lan_candidate_addrs` for DM peers; clear `lan_dial_in_flight` / `lan_candidates_exhausted`
2. `ensure_lan_tcp_listen(handover=true)` — close stale ephemeral listeners, bind fresh `/ip4/0.0.0.0/tcp/0`
3. `restart_mdns_behaviour(force=true)`
4. `notify_stream_reopen`, `clear_coord_lookup_backoff_all`, `schedule_register_presence_force`

**Triggers:** Android/Linux connectivity notify; `lan_handover_upkeep` when link down + no candidate (5s throttle); `handle_lan_interface_drift` (`lan`→`lan` key change); mobile-data→LAN `handle_lan_path_restored`.

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
LAN upkeep — nudge mDNS (link down, no candidate yet)   ← soft-only path (removed)
closed stale LAN ephemeral TCP listener (handover) every ~200ms
coord dialing … relay circuit while on LAN with no mdns dialing first
```

**Future agents — do not reintroduce:** soft mDNS restart without fresh listen; pre-throttle before `kick_lan`; port ranking; upkeep LAN re-dial from candidate cache; full `kick_lan` on every LAN dial `OutgoingConnectionError`; Flutter `p2p_notify_network_change`.

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
| **LAN / WAN connect** | Stream-first connect, route pick, coord defer | `swarm.dial`, mDNS browse, relay reserve | `ConnectionEstablished`, `OutgoingConnectionError`, mDNS `Discovered`/`Expired` |
| **Network handover** | `kick_lan` once, purge stale addrs, reopen streams | mDNS restart, ephemeral listen | Connectivity notify, profile change, relay `ListenerClosed` |
| **Coord / WAN backup** | Lookup when LAN path failed or outbox waiting | HTTP lookup, relay circuit dial | Lookup ok/404/error, bootstrap connected, reservation accepted |
| **DM stream** | Open mux, drain outbox, read-ack gate | `open_stream`, mux read/write | Stream ready, `receiver is gone`, connection closed |
| **Presence / register** | Publish when endpoints **change** | `POST /v1/register`, relay listen set | Endpoint diff, reservation accepted, handover kick |
| **Flutter UI** | Render transcript/ticks from native stores | — | Poll is **display only** — never connect/ack policy |

**Anti-pattern (any area):** A polls or sleeps because B might be done “by now”; tick loops that re-kick the same recovery (mDNS, stream reopen, coord) without a new event; tuning `N` seconds instead of wiring the subscriber.

**Allowed timers (guardrails only — all areas):**

- In-flight observation while B runs (`LAN_DIAL_IN_FLIGHT_MS`, `CIRCUIT_DIAL_IN_FLIGHT_MS`) — track B, do not replace its events
- Storm throttles (`should_issue_bootstrap_dial`, `should_routed_dial`, `should_throttle_register`)
- Keepalive ping < idle connection timeout
- Backoff after **confirmed** failure (404, refused) — not preemptive “wait before trying”

**Forbidden (regressions — often handover, same rule everywhere):** grace windows blocking all coord; tick-polled recovery without a new event; shortening timer constants as a “speed fix”; timer-driven re-dial from stale caches.

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
2. **Partial connection close** — libp2p may hold several parallel TCP paths to the same DM peer (brief mDNS burst before first connect). When one path closes, emit `PeerDisconnected` / clear the stream writer / `note_disconnected` **only if** `!swarm.is_connected(peer)` — otherwise log at debug and keep the live stream. **LAN dials are mDNS event-driven** (`handle_mdns_discovered_list`); **`dm_upkeep` → `connect_dm_peer_now` is coord/WAN only** — it must **not** re-dial LAN TCP from `peer_mdns_lan_candidate_addrs` (§ “Ephemeral LAN TCP ports”). Do **not** open parallel mDNS dials while a LAN dial is already in flight (`lan_dial_in_flight` → skip). On `OutgoingConnectionError` for a LAN TCP addr, remove that addr and fail over to the next set member once (`try_mdns_lan_failover_dial`), then `notify_coord_lookup` when exhausted. **Linux desktop** idle link timeout is **120s** (not 300s) so quiet LAN links recycle sooner after listen-port changes — still above keepalive ping interval.
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

The app accepts a **list** of coord server base URLs (today a single entry: `https://coord.ghalbol.com`; the API is an array for future redundancy). Each entry is a full **coord + relay** pair — HTTP presence plus a co-located Circuit Relay v2 node (`GET /v1/relay` on that host).

| Action | Policy |
|--------|--------|
| **Register** | Register presence (and relay circuit addr) on **every** reachable coord in the list |
| **Lookup** | When dialling a peer, try coord servers in order; **stop on first successful lookup + connect** |
| **Reconnect** | After a connection drop while internet is active, repeat lookup across the full list |
| **Coord unreachable** | Keep retrying all entries on the regular interval; **LAN (mDNS) unaffected** |

Do not substitute Kademlia DHT or public libp2p bootstrap peers when a coord lookup fails — WAN discovery requires coord/relay ([STORY.md](STORY.md)).

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
- At swarm startup, for **each** configured coord URL, `coord_runtime::fetch_all_ghalbol_relays` fetches `/v1/relay` and `network_transport::resolve_relay_bootnodes` resolves dialable bases into **both IP families** — `/ip4/<public>/tcp/<port>/p2p/<id>` **and `/ip6/<routable>/tcp/<port>/p2p/<id>`** (IPv6 sorted first; product policy is "IPv6 preferred when it works"). This is required for IPv6‑only / NAT64 mobile carriers: there the OS resolver (DNS64) synthesizes an IPv6 address for the relay's `/dns4` hostname and the literal IPv4 base has no route — keeping only IPv4 (the old behaviour) left such devices unable to reserve a circuit and therefore unreachable. `is_trusted_bootstrap_dial_addr` accepts a public IPv4 **or** a globally routable IPv6 (incl. NAT64 `64:ff9b::/96`). `issue_bootstrap_dials` dials **all** resolved families for a relay within one throttle window (happy‑eyeballs) so a preferred‑but‑unroutable family never starves the other. libp2p's relay client pins HOP to **one** bootstrap TCP link per relay, so `prune_duplicate_relay_bootstrap_connections` closes extras and keeps the best family (`relay_bootstrap_family_rank` — IPv6 on global‑v6 LAN, IPv4 on CGNAT/mobile when both connect). **Circuit reservation** is then a single `listen_on(…/p2p-circuit)` on that anchor only (`relay_circuit_listen_addr`). Do **not** issue multi‑family `listen_on` while two bootstrap TCP links are still up — HOP and circuit addr must match. The client **dials base TCP** (throttled), prunes to one link, then after identify requests the circuit. Probe-style `listen_on` runs only from `retry_stalled_relay_reservations` when bootstrap TCP is still **not** connected (not in parallel with active bootstrap dials). The resulting `/p2p-circuit` is registered in coord presence; recovery retries in § "Steady connection". **If the advertised relay TCP port is unreachable** (dev: dead bore/ngrok tunnel; prod: firewall), clients log `relay TCP unreachable`, never register on coord, and peer lookups return 404 until the tunnel is fixed.
- **Cached to disk** (`<data_dir>/ghalbol_relay.json` per coord host): a successful fetch is persisted. **Invalidate on relay TCP failure** (`Connection refused`) — client clears cache and refetches `GET /v1/relay`. **Dev bore assigns a new remote port every `run_server.sh` start** — apps must refetch after server restart; stale cache + stale ngrok JSON = wrong port. The relay **PeerId** is stable only while `<data_dir>/relay_ed25519.key` on the server is preserved — do not delete that file.

### Caching policy (P2P — avoid caches that can break connectivity)

**Default: do not cache anything that influences dial targets, peer reachability, or transport choice.** If there is even a slight chance a cache serves a stale addr, port, or path and degrades P2P, prefer live discovery (mDNS events, coord HTTP lookup, current swarm connection state).

| Allowed | Invalidation / notes |
|---------|----------------------|
| `ghalbol_relay.json` (coord relay base from `GET /v1/relay`) | Clear and refetch on relay TCP `Connection refused`; dev bore port changes each server restart |
| Short in-memory throttles (`should_routed_dial`, `lan_dial_in_flight`, per-relay reserve throttle) | Time-bounded; not substitute for live reachability |

| **Do not add** | Why |
|----------------|-----|
| Cached coord lookup multiaddrs for urgent reconnect | Stale relay/RFC1918 addrs race live mDNS and stall LAN (removed `coord_cached_dial_addrs`) |
| `peer_mdns_lan_candidate_addrs` (in-memory live set) | Add on mDNS `Discovered`; remove per-addr on `Expired` / dial fail. **Upkeep must not re-dial from this set.** Failover only on immediate dial error — not a timer cache. |
| Cached “peer is on LAN” without mDNS/`Expired` refresh | Use `peers_on_local_lan` + per-addr `note_peer_mdns_lan_addr_expired`; drop LAN pref when candidate set empty |
| Port-ranking heuristics (highest port, preferred addr, TTL age) | Ephemeral ports change every restart; ranking masked stale-cache bugs — see § “Ephemeral LAN TCP ports” |
| `dm_upkeep` LAN re-dials to “refresh” a cached port | LAN connect is **event-driven** only; upkeep is coord/WAN |
| Dart-side dial/lookup caches | All WAN/LAN routing lives in Rust (`chat_server.rs`, `coord_runtime.rs`) |

When adding new state, ask: “If this value is wrong for 30s, does chat break?” If yes, do not cache — compute from live signals or refetch.
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
| **One bootstrap TCP per coord relay** on the best IP family — libp2p relay HOP uses `directly_connected_peers.first()` only | `prune_duplicate_relay_bootstrap_connections` |
| Bootstrap dials may race v4+v6; reservation `listen_on` is `…/p2p-circuit` on the **kept** anchor only | `relay_circuit_listen_addr` + `relay_reservation_circuit_addrs` |
| Bootstrap relay **dials** throttled per relay (no dial storm on CGNAT) | `issue_bootstrap_dials` / `should_issue_bootstrap_dial` |
| CGNAT/mobile: probe-style relay reservation when bootstrap TCP pending | `try_ghalbol_probe_style_circuit_listen` at startup + `retry_stalled_relay_reservations` |
| Outbound peer relay dials after coord lookup are **not** gated on own `relay_circuit_listening` | `dial_dm_peer_addr` + `should_routed_dial` only |
| LAN discovery upgrades a relay-only link (additive, never tears down WAN); LAN loss drops the LAN pref immediately + re-kicks WAN | `dial_mdns_peer` / `dial_lan_upgrade` / `note_peer_mdns_lan_addr_expired` / `forget_peer_on_local_lan` |
| LAN dials from mDNS events only; upkeep does not re-dial cached LAN ports | `handle_mdns_discovered_list`; `connect_dm_peer_now` coord/WAN only |

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
9. **Skipping relay TCP dial for the coord relay** — client must dial `GET /v1/relay` base addr (throttled), then reserve; on CGNAT also use probe-style `listen_on` when bootstrap is not connected yet (§ “CGNAT / mobile-data relay reservation”).
10. **Treating coord HTTP OK as WAN OK** — `GET /v1/relay` 200 with unreachable relay TCP → endless `GET /v1/peers/…` 404; fix bore/firewall first.
11. **Reintroducing static `bootstrap_peers` for WAN** — `bootstrap_peers: []` is intentional; only coord relay from `/v1/relay` is the WAN dial target.
12. **Uncoordinated bootstrap relay dial spam** — refetch + WAN recovery + redial calling `swarm.dial` every 1–2s without `should_issue_bootstrap_dial` prevents bootstrap TCP from completing on mobile-data; log shows many `coord relay dial` lines, never `bootstrap connection`.
13. **Removing CGNAT probe reservation** — `try_ghalbol_probe_style_circuit_listen` at startup / when `!any_bootstrap_connected` is required for phones; Wi‑Fi-only tests hide the regression.
14. **One-sided relay OK** — Linux `reservation accepted` while Android stuck on `CGNAT listen addr only` means chat will not work; both peers must register on coord.
15. **Blocking peer relay dials until own circuit listens** — `skip relay dial … self relay circuit not ready yet` after `coord_lookup_peer ok` stalls WAN 30–40s on CGNAT; peer outbound dials only need coord relay bootstrap TCP + peer registered. See § “Outbound peer relay dials vs own reservation”.
16. **Racing coord relay dials against mDNS LAN on Wi‑Fi before first connect** — gating `should_defer_coord_relay_for_lan` on `connected == true` causes relay + LAN TCP to cancel each other; endless ~15s mDNS retries, no `peer_connected`. See § “LAN relay vs mDNS race”.
17. **P2P dial/lookup caches** — coord lookup addr cache, frozen mDNS LAN addr, upkeep re-dials from `peer_mdns_lan_candidate_addrs`, or Dart-side routing cache. Prefer live mDNS events + coord HTTP; only `ghalbol_relay.json` with TCP-failure invalidation. See § “Caching policy (P2P)”, § “Ephemeral LAN TCP ports”.
18. **Port guessing / ranking heuristics** — highest-port-wins, “preferred” mDNS addr, TTL-based pick, or `nc` probes instead of reading mDNS lifecycle + `Native/flow` listen_addrs. See § “Ephemeral LAN TCP ports”.
19. **Soft mDNS-only Wi‑Fi switch recovery** — `restart_mdns_behaviour` without `ensure_lan_tcp_listen(handover=true)` and candidate purge; or pre-consuming `should_run_lan_recovery` then skipping full `kick_lan`. Symptom: endless `LAN upkeep — nudge mDNS`, no `mdns discovered`. See § “LAN stability — cold start and Wi‑Fi toggle”.

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
| 2026-06-16 | **LAN stability (verified short test):** cold-start LAN and Wi‑Fi off/on on same subnet recover on Linux + Android. Fixes: `linux_network.rs` (sysfs operstate → `notify_network_change`); `platform_wifi_linked`; full `kick_lan` on connectivity notify and in `lan_handover_upkeep` (not soft mDNS-only); `dm_down_on_lan` on 1s poll when streams down. § “LAN stability — cold start and Wi‑Fi toggle”. Removed port ranking / upkeep LAN re-dial (earlier). |
| 2026-06-16 | **Event-driven async (general rule):** § “Event-driven async — avoid assumed timers”; A/B subscriber model for connect, handover, lookup, reserve, stream, register. AGENTS + DESIGN.md aligned. |
| 2026-06-16 | **Architecture:** Android Wi‑Fi probe in Rust (`android_network.rs`); `:p2p` Kotlin registers callbacks only; removed Flutter `p2p_notify_network_change`. |
| 2026-06-17 | **Hybrid coord presence (P4–P6):** clients POST public/LAN tcp only; `ghal_bol_server` relay upserts `/p2p-circuit` on reservation; clients poll coord when register payload empty; keep `coord_registered` during handover when relay circuit still visible; throttle coord lookup during handover + degraded HTTP; do not drop WAN relay when stale direct counters linger after `left LAN`. |
| 2026-06-16 | **LAN connect model (supersedes port-ranking experiments):** mDNS event-driven dial only; `connect_dm_peer_now` coord/WAN-only; removed `rank_mdns_lan_tcp_candidates` / highest-port heuristics; 45s handover grace defers coord while waiting for fresh `Discovered`. § “Ephemeral LAN TCP ports”, § “LAN relay vs mDNS race”. |
| 2026-06-15 | **Ephemeral LAN ports / stale mDNS cache:** documented ephemeral TCP + candidate-set lifecycle; stopped upkeep re-dials to stale ports. § “Ephemeral LAN TCP ports”. |
| 2026-06-15 | **Wi‑Fi return handover:** `has_rfc1918_on_wifi` + `lan_restored`; soft handover on mobile-data→LAN; immediate mDNS purge on `lan → mobile-data`. |
| 2026-06-15 | **FORBIDDEN hub session patch (reverted):** `lastApplySucceeded` / per-frame session retry — broke P2P. Never reintroduce. DESIGN.md § “FORBIDDEN — reverted 2026-06-15”. |
| 2026-06-14 | **Stream-first symmetric connect:** canonical connect model from the original serverless build (seconds to connect, one stream per contact, single upkeep owner). Coord/relay/mDNS are discovery inputs only — must not race parallel dials. § “Stream-first symmetric connect”; DESIGN.md § same. Removed incorrect “coord relay first on outbox” dial guidance. |
| 2026-06-13 | **LAN relay vs mDNS race (regression fix):** on Wi‑Fi, defer coord relay dials while a **direct LAN dial is in flight** — **before** first connect. Removed coord lookup addr cache; mDNS uses live candidate list per peer. § “LAN relay vs mDNS race”, § “Caching policy (P2P)”. |
| 2026-06-11 | **Bootstrap TCP prune (libp2p relay HOP pin):** happy-eyeballs left **two** coord-relay TCP links (v6 then v4); libp2p's relay client sends all HOP (reserve + routed DM dial) on `directly_connected_peers[relay].first()` only. When v6 connected first on mobile-data but could not carry HOP, v4 bootstrap was ignored — server saw `client connected` ×2, no `reservation`/`circuit` events. `prune_duplicate_relay_bootstrap_connections` keeps one link (IPv4 on mobile-data path, IPv6 when LAN has global v6); reservation uses that anchor only. |
| 2026-06-11 | **Relay reservation:** dual-family bootstrap **dials** kept; `prune_duplicate_relay_bootstrap_connections` keeps one TCP on best family (`relay_bootstrap_family_rank`); one `listen_on(…/p2p-circuit)` on that anchor. No startup probe while bootstrap dials run; `relay_reservation_active` gates on accepted reservation only. |
| 2026-06-11 | **Relay reservation regression fix:** with `bootstrap_relay_addr` set, `relay_reservation_circuit_addrs` returned the base TCP addr without `/p2p-circuit`, so `listen_on` failed (`relay reserve listen …` empty error). Fixed via `relay_circuit_listen_addr`. |
| 2026-06-11 | Dual-family bootstrap **dial** retained (`issue_bootstrap_dials`); dual `listen_on` per family removed once anchor exists — HOP is single-connection. |
| 2026-06-10 | **Outbound peer relay dials (regression fix):** removed gate that skipped coord peer relay dials when own `relay_circuit_listening` was false on CGNAT — caused `skip relay dial … self relay circuit not ready yet` for ~40s while lookup succeeded. Peer circuit dials use `should_routed_dial` only; own reservation stays on bootstrap/probe/reserve path. § “Outbound peer relay dials vs own reservation”. |
| 2026-06-10 | **WAN startup latency:** dial coord relay + CGNAT probe before `bootstrap_publishable_listen`; process `ConnectionEstablished` during listen wait (bootstrap TCP was completing while events were ignored → ~minute relay delay on mobile). Shorter mobile listen wait; faster CGNAT probe throttle; probe `listen_on` Err clears retry state; cap coord 404 backoff for DM contacts at 3s. |
| 2026-06-09 | **CGNAT relay reservation regression (recurring):** § “CGNAT / mobile-data relay reservation” — asymmetric symptom (Wi‑Fi OK, phone 404), bootstrap **dial storm** vs missing probe `listen_on`, required fixes (`issue_bootstrap_dials`, startup WAN recovery, `retry_stalled` probe path). Steady-connection item 4 + invariants + AI handoff items 12–14. |
| 2026-06-09 | **WAN dev regression docs + relay dial path:** § “Naming — bootstrap in logs”, § “WAN prerequisites”, symptom→cause table. Client dials coord relay TCP before reservation; clears `ghalbol_relay.json` on refused dial. Dev bore port changes each `run_server.sh` run — document refetch requirement. |
| 2026-06-09 | **STORY alignment — coord-required WAN, no KAD/bootstrap discovery:** WAN peer discovery requires configured coord/relay servers; Kademlia DHT and public libp2p bootstrap peers are not fallbacks. LAN (mDNS) still works when coord is down. Added § "Multiple coord / relay servers". See [STORY.md](STORY.md). |
| 2026-06-06 | **Immediate LAN shift + fast WAN fallback (STORY):** mDNS `Discovered` now upgrades a relay-only link to a direct LAN connection (`dial_lan_upgrade`, `PeerCondition::NotDialing`, per-peer throttle; additive, never tears down WAN); per-connection direct/relay tracking added (`peers_direct_conns`). mDNS `Expired` drops the LAN preference immediately (no 180s TTL wait) and re-kicks WAN discovery. See § "Immediate LAN shift + fast WAN fallback". |
| 2026-06-06 | **Relay secp256k1 fix (root cause of WAN failure):** `ghal_bol_server`'s `libp2p` was missing the `secp256k1` feature, so the relay dropped every secp256k1 client (i.e. every real device) mid Noise handshake (`Decode(UnexpectedEof)` → circuit listener `addrs=[]` → `coord_registered=false`). Added `secp256k1` (+`ed25519`) to the server. `examples/relay_probe.rs` gained `PROBE_SECP256K1=1` because the ed25519-only probe masked the bug. Removed debug-only scaffolding committed during the hunt (relay-reservation repro test module, `keycheck` example). |
| 2026-06-06 | **Ghal Bol relay co-located with coord** (`ghal_bol_server/src/relay.rs`, `GET /v1/relay`): clients reserve a circuit on our own reliably-granting relay (preferred over public IPFS bootstraps) and register that `/p2p-circuit` in presence — fixes WAN for NAT⇄CGNAT pairs where neither side is directly reachable. |
| 2026-06-06 | WAN regression fix: relay reservations fan out to **all** eligible bootstraps again (per-relay throttle), reverting the one-at-a-time scheme that serialized behind a stuck pending reservation and stalled WAN for minutes. |
| 2026-06-05 | Steady-connection hardening: keepalive `ping`, urgent DM reconnect (no 404 backoff), one-at-a-time relay reservation. |
| 2026-05-31 | Canonical transport doc; libp2p confirmed as production stack. |
