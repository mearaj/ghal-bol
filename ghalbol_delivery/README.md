# ghalbol_delivery

Temporary encrypted message mailbox for Ghal Bol text delivery.

Design: [docs/GHALBOL_DELIVERY.md](../docs/GHALBOL_DELIVERY.md)

## Home deploy (`delivery.ghalbol.com`)

See [deploy/DELIVERY_HOME.md](deploy/DELIVERY_HOME.md) and [deploy/README.md](deploy/README.md).

```bash
./ghalbol_delivery/deploy/install_delivery_home.sh
./ghalbol_delivery/deploy/enable_delivery_https.sh
./ghalbol_delivery/deploy/verify_delivery.sh
```

Production app URL: `GHALBOL_DELIVERY_URL=wss://delivery.ghalbol.com:55003`

## Run (dev)

```bash
cargo run -p ghalbol_delivery
```

Default listen: `0.0.0.0:8770`. Data dir: `~/.local/share/com.ghalbol.delivery/ghalbol_delivery/`.

## Operator CLI (migration)

```bash
ghalbol_delivery mailbox-stats
ghalbol_delivery export-mailbox --out mailbox-export.tar.zst
ghalbol_delivery import-mailbox --in mailbox-export.tar.zst --replace
```

## Env

| Variable | Default |
|----------|---------|
| `GHALBOL_DELIVERY_LISTEN` | `0.0.0.0:8770` |
| `GHALBOL_DELIVERY_DATA_DIR` | `~/.local/share/com.ghalbol.delivery/ghalbol_delivery/` |
| `GHALBOL_DDNS_CREDENTIALS` | unset — enables in-process GoDaddy DDNS when set |
| `GHALBOL_DELIVERY_INSTANCE_ID` | hostname — exposed in `/health` for ops |

## Test

```bash
cargo test -p ghalbol_delivery
```
