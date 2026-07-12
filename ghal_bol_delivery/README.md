# ghal_bol_delivery

Temporary encrypted message mailbox for Ghal Bol WAN text delivery.

Design: [docs/GHAL_BOL_DELIVERY.md](../docs/GHAL_BOL_DELIVERY.md)

## Home deploy (`delivery.ghalbol.com`)

See [deploy/DELIVERY_HOME.md](deploy/DELIVERY_HOME.md) and [deploy/README.md](deploy/README.md).

```bash
./ghal_bol_delivery/deploy/install_delivery_home.sh
./ghal_bol_delivery/deploy/enable_delivery_https.sh
./ghal_bol_delivery/deploy/verify_delivery.sh
```

Production app URL: `GHAL_BOL_DELIVERY_URL=wss://delivery.ghalbol.com:55003`

## Run (dev)

```bash
cargo run -p ghal_bol_delivery
```

Default listen: `0.0.0.0:8770`. Data dir: `~/.local/share/com.ghal_bol.delivery/ghal_bol_delivery/`.

## Operator CLI (migration)

```bash
ghal_bol_delivery mailbox-stats
ghal_bol_delivery export-mailbox --out mailbox-export.tar.zst
ghal_bol_delivery import-mailbox --in mailbox-export.tar.zst --replace
```

## Env

| Variable | Default |
|----------|---------|
| `GHAL_BOL_DELIVERY_LISTEN` | `0.0.0.0:8770` |
| `GHAL_BOL_DELIVERY_DATA_DIR` | `~/.local/share/com.ghal_bol.delivery/ghal_bol_delivery/` |
| `GHAL_BOL_DDNS_CREDENTIALS` | unset — enables in-process GoDaddy DDNS when set |
| `GHAL_BOL_DELIVERY_INSTANCE_ID` | hostname — exposed in `/health` for ops |

## Test

```bash
cargo test -p ghal_bol_delivery
```
