# Documentation

| Document | Contents |
|----------|----------|
| [DESIGN.md](DESIGN.md) | **Canonical** layers, **truthful UI ticks**, message state, room open/close ordering, leave backlog, transcript keys, read-ack confirm loop, **`is_known` / `is_blocked`** contact trust UI, anti-patterns |
| [GHAL_BOL_DM_MSG_V1.md](GHAL_BOL_DM_MSG_V1.md) | Wire format, `ack_received` / `ack_read`, upkeep, poll events, implementer checklist |
| [../README.md](../README.md) | System design: identity, networking, sync model, coordination server |
| [COORDINATION_SERVER.md](COORDINATION_SERVER.md) | Run and test `ghal_bol_server` (API, coord_client, prod `coord.ghalbol.com`, deploy smoke) |
| [COMMUNICATION_TIERS.md](COMMUNICATION_TIERS.md) | Tier 1 direct P2P, Tier 2 peer relay, Tier 3 paid backup — priority and implementation status |
| [WHAT_GHAL_BOL_SOLVES.md](WHAT_GHAL_BOL_SOLVES.md) | Product vision: problems addressed and what the project is not |
| [GHAL_BOL_URI_SCHEME.md](GHAL_BOL_URI_SCHEME.md) | Connect invites: `ghalbol.com`, `ghalbol://` |
| [PEER_DISCOVERY.md](PEER_DISCOVERY.md) | Invites (`ghalbol.com`), coordination lookup, direct QUIC sync |
| [TRANSPORT.md](TRANSPORT.md) | **libp2p** transport stack, discovery, invariants |
| [PRODUCTION_RELEASE.md](PRODUCTION_RELEASE.md) | **P0 / P1 / P2** ship checklist (APK, ship test, systemd, Play Store) |
| [PRIVACY_POLICY.md](PRIVACY_POLICY.md) | Privacy policy draft (host at public URL for Play Console) |
| [PLAY_STORE_LISTING.md](PLAY_STORE_LISTING.md) | Play Store listing and Data safety draft |
| [IDENTITY.md](IDENTITY.md) | Local identity: create vs import, export/import backup, reveal private key, ownership model |
| [PREMIUM_SERVICES.md](PREMIUM_SERVICES.md) | Optional Tier 3 services, payment rails, membership separate from messaging keys |
| [TODO.md](TODO.md) | **Product backlog** — wish list only; design lives in other docs |
| [WEB_SITE.md](WEB_SITE.md) | **ghalbol.com** — Firebase Hosting, `/connect/…` handoff, `/download/linux`, Linux tarball in `web/downloads/` |

**Public site:** `https://ghalbol.com` — home, Play Store, Linux download, invite pages. **Coordination (separate host):** `https://coord.ghalbol.com`.

**Invite URLs:** `https://ghalbol.com/connect/<public_key_hex>` and `ghalbol://connect/<public_key_hex>`.
