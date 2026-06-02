# ghal_bol_server

Production **Tier 1 coordination** server for Ghal Bol: signed peer registration, SQLite presence, endpoint lookup.

Does **not** store message bodies or transcripts. This is **not** Tier 3 paid backup relay (see [docs/COMMUNICATION_TIERS.md](../docs/COMMUNICATION_TIERS.md)).

## Run

```bash
cargo run -p ghal_bol_server
```

Defaults:

| Setting | Default |
|---------|---------|
| Listen | `127.0.0.1:8765` |
| SQLite | `~/.local/share/com.ghalbol/ghalbol_server/coord.db` |

Same data root as the Flutter app and `ghal_bol` (`com.ghalbol`).

```bash
GHAL_BOL_SERVER_LISTEN=0.0.0.0:8765 RUST_LOG=info cargo run -p ghal_bol_server
```

`GHAL_BOL_SERVER_DB` may be a **file** (`…/coord.db`) or **directory** (`…/ghalbol_server`).

## Server smoke (no Flutter)

```bash
./ghal_bol_server/deploy/smoke_coord.sh
COORD_URL=http://127.0.0.1:8765 ./ghal_bol_server/deploy/smoke_coord.sh
```

Manual CLI: `cargo build -p ghal_bol_server --release` then `./target/release/coord_client http://127.0.0.1:8765 demo-two-peers`. See [docs/COORDINATION_SERVER.md](../docs/COORDINATION_SERVER.md).

## Test

**Production E2E (real process + TCP + disk SQLite):**

```bash
cargo test -p ghal_bol_server --test e2e_production
```

Spawns the compiled `ghal_bol_server` binary, binds an ephemeral port, uses `reqwest` over HTTP.

**Fast handler checks (in-process, no TCP):**

```bash
cargo test -p ghal_bol_server --test http_api
```

**All:**

```bash
cargo test -p ghal_bol_server
```

## HTTP API (v1)

| Method | Path | Body |
|--------|------|------|
| GET | `/health` | — (`database: true` when SQLite answers) |
| POST | `/v1/register/challenge` | `{ "public_key_hex": "<66 hex>" }` |
| POST | `/v1/register` | `public_key_hex`, `nonce_hex`, `signature_hex`, `endpoints[]`, optional `ipv4` / `ipv6` / `transport_capabilities` |
| POST | `/v1/heartbeat` | `{ "public_key_hex": "<66 hex>" }` |
| GET | `/v1/peers/{public_key_hex}` | — |
| GET | `/v1/peers` | — online peers (heartbeat within TTL) |

Registration signature:

```text
ghal_bol:register:v1
<nonce_hex>
<public_key_hex_lowercase>
```

SHA-256 digest → secp256k1 ECDSA (DER).

## Environment

| Variable | Default |
|----------|---------|
| `GHAL_BOL_SERVER_LISTEN` | `127.0.0.1:8765` |
| `GHAL_BOL_SERVER_DB` | `~/.local/share/com.ghalbol/ghalbol_server/coord.db` |
| `GHAL_BOL_SERVER_CHALLENGE_TTL_SECS` | `120` |
| `GHAL_BOL_SERVER_PRESENCE_TTL_SECS` | `90` |
| `GHAL_BOL_SERVER_PURGE_INTERVAL_SECS` | `30` |

## Deploy

[deploy/README.md](deploy/README.md): `./deploy/run_server.sh`, **nginx + TLS** for production (`coord.ghalbol.com`), or **ngrok** for dev.

## Checklist

- [x] `cargo test -p ghal_bol_server --test e2e_production` green
- [x] Production coord live — `https://coord.ghalbol.com/health`
- [x] `COORD_URL=https://coord.ghalbol.com ./deploy/smoke_coord.sh` passes
- [ ] VM systemd enabled and survives reboot
- [ ] App ship test: two phones on mobile data via prod coord URL
