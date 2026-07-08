# Coordination server (`ghal_bol_server`)

**Tier 1 only** — presence and endpoint discovery. No message bodies or transcripts.

It also runs a co-located **libp2p Circuit Relay v2** node for NAT/CGNAT traversal (transport helper; peers dial each other via `/p2p-circuit` multiaddrs registered on coord — not a message store). See [TRANSPORT.md](TRANSPORT.md) § "Ghal Bol relay".

```text
Peer A / B  →  register / heartbeat  →  ghal_bol_server (SQLite)  →  GET /v1/peers/{[algo:]hex}  →  dial /p2p-circuit
                 └─ reserve circuit on relay ─→  GET /v1/relay  →  server upserts /p2p-circuit on coord
```

## Client (`ghal_bol`)

After unlock: configure the **coord server list** (today a single URL; the API is an array for future redundancy). On P2P listen: **register + heartbeat on every reachable server** in the list. Before dial: **lookup across the list** — stop on first successful result for that attempt; on reconnect after a drop, repeat the full list.

Set URLs in `ghal_bol_ui/env/.env.development` (debug) or `env/.env.production` (release) via `GHAL_BOL_COORD_URLS` (JSON array or comma-separated). No hardcoded coord URLs in the app binary.

**WAN policy:** coord + co-located relay are **required** for internet peer discovery. WAN dials use explicit **`/p2p-circuit`** multiaddrs from `GET /v1/peers/{pk}` — **not** DCUtR hole-punch (DCUtR is disabled when coord is configured; see [TRANSPORT.md](TRANSPORT.md) § Stream-first). When coord is unreachable, LAN (mDNS) still works; the node keeps retrying all configured servers. Do **not** fall back to Kademlia DHT or public libp2p bootstrap peers for WAN peer lookup — **libp2p remains** for transport (relay circuit, mDNS, streams).

### Client register & heartbeat policy (`coord_runtime.rs`)

**Goal (product):** stay registered with a **correct, dialable** WAN presence — steady and accurate, not spam. The client must **never** believe it is registered when coord does not list a valid endpoint for the peer.

| Trigger | Action |
|---------|--------|
| Publishable endpoint set **changed** (public TCP and/or relay circuit) | Full `POST /v1/register` on **all** configured coord URLs (`schedule_register_presence_force`) |
| Last register **failed** or never succeeded | Retry register (min gap **2s** between attempts) |
| Relay **reservation accepted** (public TCP path) or **network handover** | Force re-register when endpoints change |
| **Presence stale** — no successful heartbeat/self-lookup in **~70s** (`PRESENCE_STALE_MS`; server row TTL ~90s) | Force re-register |
| **`:p2p` / daemon restart** (`p2p_start`) | Re-fetch `GET /v1/relay`, re-reserve if needed, register when endpoints known — do not trust prior process state |
| Endpoints **unchanged** and recently registered | **Throttle** full register (min gap **10s** when `coord_registered`); use **`POST /v1/heartbeat` every 25s** instead |

**Truthfulness:** `coord_registered` in logs is set only after HTTP success or relay-presence self-lookup (`GET /v1/peers/{self}` / relay presence poll). CGNAT-only peers may show circuit via **relay server upsert** before client `POST` succeeds — see § Hybrid presence below.

**Lookup (peers):** try configured coord servers **in order**; **stop on first successful** lookup for that dial attempt. After a peer disconnect with internet still up, **repeat lookup from the first server**.

Implementation: `should_throttle_register`, `spawn_register_presence_inner`, `coord_register_tick` in `ghal_bol/src/coord_runtime.rs`. Broader connectivity rules: [TRANSPORT.md](TRANSPORT.md) § **Connectivity lifecycle**.

## Run server

**Home** — `./ghal_bol_server/deploy/install_coord1_home.sh` + `./ghal_bol_server/deploy/verify_coord1.sh`. See [COORD1_HOME.md](../ghal_bol_server/deploy/COORD1_HOME.md).

**GCP** — `./ghal_bol_server/deploy/deploy_server.sh`.

**Loopback smoke:**

```bash
cargo run -p ghal_bol_server
curl -s http://127.0.0.1:8765/v1/relay | jq
```

## Production (`coord.ghalbol.com`)

Public HTTPS coordination is served by **nginx + TLS** in front of `ghal_bol_server` on `127.0.0.1:8765`. Relay is **not** behind nginx — clients dial `coord.ghalbol.com:4002` directly. Example config: [ghal_bol_server/deploy/nginx-coord.conf](../ghal_bol_server/deploy/nginx-coord.conf).

App builds bundle the URL via `ghal_bol_ui/env/.env.production`:

```bash
GHAL_BOL_COORD_URLS=["https://coord.ghalbol.com"]
```

Smoke against production:

```bash
COORD_URL=https://coord.ghalbol.com ./ghal_bol_server/deploy/smoke_coord.sh
```

## Home (`coord1.ghalbol.com`)

Same nginx pattern as GCP for **coord HTTP**. **GoDaddy DDNS** runs **in-process** inside `ghal_bol_server` (`GHAL_BOL_DDNS_CREDENTIALS`, poll on start + every 5 min). One-shot manual update: `godaddy-ddns.sh`.

**Relay:** fixed TCP **55002** (same model as GCP `:4002`, but home routers often block 4002). `install_coord1_home.sh` sets `GHAL_BOL_RELAY_LISTEN=0.0.0.0:55002`, `GHAL_BOL_RELAY_PUBLIC_HOST=coord1.ghalbol.com`. **Router:** forward **8443** (HTTPS) and **55002** (relay) to the coord1 host.

```bash
./ghal_bol_server/deploy/install_coord1_home.sh
./ghal_bol_server/deploy/verify_coord1.sh
```

```bash
GHAL_BOL_COORD_URLS=["https://coord1.ghalbol.com:8443"]
```

See [COORD1_HOME.md](../ghal_bol_server/deploy/COORD1_HOME.md).
## HTTP API (v1)

### Identity wire (path + JSON `public_key_hex`)

The **same identity wire** appears in JSON bodies (`public_key_hex`) and as the **lookup path segment**. **Algorithm prefix is optional only for `secp256k1`** (bare hex); `ed25519`, `ecdsa-p256`, `ml-dsa-65`, etc. **must** include `algorithm:`.

```text
GET /v1/peers/{[algo:]public_key_hex}
```

| Form | Meaning | Example lookup path |
|------|---------|---------------------|
| Bare hex (no `:`) | **Implicit `secp256k1`** | `/v1/peers/02a1b2…` |
| `algorithm:hex` | Explicit algorithm | `/v1/peers/ed25519%3A9f86…` (`:` → `%3A`) |

Server-side: `ghal_bol_server/src/identity.rs` → `normalize_identity_wire()` on **register**, **heartbeat**, **lookup**, relay `pk=` binding, and SQLite primary key. Explicit `secp256k1:…` normalizes to bare hex on store.

Client-side: `ghal_bol/src/coord.rs` uses `normalize_contact_identity_wire()` (same parse rules). Lookup URL-encodes the wire (`ed25519:…` → `ed25519%3A…`).

**Transport `endpoints[]`** (scheme/host/port or libp2p multiaddr) are dial addresses — **not** identity strings. They do not carry an algorithm prefix.

| Method | Path |
|--------|------|
| GET | `/health` |
| POST | `/v1/register/challenge` |
| POST | `/v1/register` |
| POST | `/v1/heartbeat` |
| GET | `/v1/peers/{[algo:]public_key_hex}` | Path segment = identity wire (bare hex = implicit `secp256k1`; prefix `:` as `%3A`) |
| GET | `/v1/peers` |
| GET | `/v1/relay` |
| GET | `/v1/relay?remap=true` | UPnP-dynamic relay only (not shipping on coord1): after client bootstrap TCP failure — remove stale WAN port, map fresh (bool query — **`true`/`false`**, not `1`/`0`; storm-throttled) |

Register signature: canonical bytes `ghal_bol:register:v1\n<nonce_hex>\n<identity_wire>` (identity wire lowercased), signed with the identity key — **secp256k1** ECDSA DER, **ed25519**, **ecdsa-p256** DER, or **ml-dsa-65** per algorithm (`ghal_bol_server/src/auth.rs`).

**Hybrid presence (shipping):** `POST /v1/register` accepts **client** endpoints only: **`tcp` / `quic` with globally routable IPv4** (the peer’s own inbound DM listen). **Rejected (400):** `/p2p-circuit`, RFC1918 LAN, CGNAT-only, relay bootstrap host:port from `GET /v1/relay`. The co-located relay **upserts** `/p2p-circuit` when the client’s reservation is accepted (identify `agent_version` `ghal_bol/<ver>;pk=<identity_wire>` — bare secp256k1 hex or `algorithm:hex`). When the reservation ends, the server removes **only** the circuit row — public-TCP rows from `POST` stay. See [TRANSPORT.md](TRANSPORT.md) § “Hybrid coord presence”.

`GET /v1/relay` → `{ enabled, peer_id, addrs }` — the co-located relay's stable PeerId and dialable base multiaddrs (clients append `/p2p/<peer_id>/p2p-circuit`). `enabled:false` or empty `addrs` when `GHAL_BOL_RELAY_PUBLIC_HOST` / `GHAL_BOL_RELAY_PUBLIC_ADDRS` are unset or relay is disabled.

**Server log (healthy WAN):** `relay reservation ACCEPTED` → `coord presence registered from relay reservation` → optional `peer registered` when client POSTs LAN/public tcp.

## Troubleshooting

### Interpreting coord HTTP access logs

| Pattern | Likely cause | Action |
|---------|--------------|--------|
| `GET /v1/relay` 200, many `GET /v1/peers/…` 404, **no** register | Relay TCP unreachable or clients stuck waiting for circuit | Home: `./ghal_bol_server/deploy/verify_coord1.sh` (relay `:55002` must pass). GCP: `nc -zv coord.ghalbol.com 4002` |
| `GET /v1/health` 200, `/v1/relay` empty addrs | Server up but relay disabled or failed to bind | Home: `journalctl --user -u ghal-bol-server-coord1`; GCP: set `GHAL_BOL_RELAY_PUBLIC_HOST` |
| `peer registered` in **server** logs but lookup 404 | TTL expired (~90s) or wrong coord URL in app | Heartbeat/register failing; check app `coord_registered` |
| `peer registered` but client `peer_on_coord_no_dial_addrs` | Row has relay bootstrap `tcp` or LAN-only — no `/p2p-circuit` | Phone lost reservation; client must not POST relay IP:port; wait for `reservation ACCEPTED` + server circuit upsert |
| `relay circuit DENIED` … `NoReservation` | Destination peer has no active relay reservation | Remote `:p2p` dropped reservation (background/LAN handover); remote must re-reserve |

### Session checklist

1. Coord server running (`ghal-bol-server-coord1` or GCP systemd)  
2. `curl -s http://127.0.0.1:8765/v1/relay | jq` — enabled + addrs  
3. **Relay TCP reachable** — GCP: `nc -zv coord.ghalbol.com 4002`. Home coord1: `./ghal_bol_server/deploy/verify_coord1.sh` (relay **55002**)
4. Rebuild native + restart apps after server identity or relay config change

When testing app traffic against your local server (not production), set `GHAL_BOL_COORD_URLS` to a reachable `http://…:8765` URL and restart the app.

Full detail: [ghal_bol_server/deploy/README.md](../ghal_bol_server/deploy/README.md) § “Regression prevention”, [TRANSPORT.md](TRANSPORT.md) § “WAN prerequisites”.

## Environment

| Variable | Default |
|----------|---------|
| `GHAL_BOL_SERVER_LISTEN` | `127.0.0.1:8765`. Dual-stack: the server also binds the counterpart-family wildcard (`[::]:<port>`, IPv6 `V6ONLY`) on the same port so coord HTTP is reachable over both IPv4 and IPv6; a missing stack logs a warning and continues single-stack |
| `GHAL_BOL_SERVER_DB` | `~/.local/share/com.ghalbol.coord/ghalbol_server/coord.db` |
| `GHAL_BOL_SERVER_PRESENCE_TTL_SECS` | `90` |
| `GHAL_BOL_RELAY_ENABLE` | `1` (set `0` to disable the relay node) |
| `GHAL_BOL_RELAY_LISTEN` | `0.0.0.0:4002` (GCP default). Home coord1: **`0.0.0.0:55002`** via `install_coord1_home.sh`. Raw TCP — **open this port** on the router/firewall; not proxied by nginx |
| `GHAL_BOL_RELAY_PUBLIC_HOST` | unset → advertises **both** `/dns6/<host>/tcp/<port>` and `/dns4/<host>/tcp/<port>` (IPv6 first; e.g. `coord.ghalbol.com`) so clients can reserve over either family. Native IPv6 needs an `AAAA` record; IPv4-only/NAT64 clients map the host themselves |
| `GHAL_BOL_RELAY_PUBLIC_ADDRS` | unset → comma-separated dialable multiaddrs (overrides `_PUBLIC_HOST`) |
| `GHAL_BOL_RELAY_MAX_CIRCUIT_BYTES` | `0` (unlimited per circuit; set e.g. `2147483648` for 2 GiB cap) |
| `GHAL_BOL_RELAY_MAX_CIRCUITS_PER_PEER` | `16` |

Production VM egress cap (Linux **tc**): `GHAL_BOL_RELAY_EGRESS_MBIT` in `deploy_server.sh` → rendered into [relay-egress-cap.service](../ghal_bol_server/deploy/relay-egress-cap.service). See [GCP.md](../ghal_bol_server/deploy/GCP.md).

## `coord_client`

```bash
cargo build -p ghal_bol_server --release
./target/release/coord_client http://127.0.0.1:8765 demo-two-peers
```

`-k` = skip TLS verify (self-signed / dev HTTPS).

## Local smoke

```bash
cargo run -p ghal_bol_server
COORD_URL=http://127.0.0.1:8765 ./ghal_bol_server/deploy/smoke_coord.sh
```

Set `GHAL_BOL_COORD_URLS` in `ghal_bol_ui/env/.env.development` (default: `https://coord.ghalbol.com`). Rebuild after changes.

**Rebuild native after Rust changes** (quit app first):

```bash
./scripts/sync_ghal_bol_native_for_flutter.sh   # Linux
./scripts/pack_android_workspace_jni_libs.sh    # Android
cd ghal_bol_ui && flutter run
```

Two-device test: server running → desktop `flutter run` + QR → phone scan → send both ways. Same Wi‑Fi uses mDNS; different subnets need coord lookup (URLs from env).

## Related

- [TRANSPORT.md](TRANSPORT.md) — WAN/LAN dial policy, invites, multiple coord servers
- [ghal_bol_server/README.md](../ghal_bol_server/README.md)
- [ghal_bol_server/deploy/README.md](../ghal_bol_server/deploy/README.md)
