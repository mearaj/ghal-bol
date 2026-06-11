# Documentation

Canonical index — read [DESIGN.md](DESIGN.md) before changing P2P, acks, invites, or persistence.

| Document | Contents |
|----------|----------|
| [DESIGN.md](DESIGN.md) | **Canonical** layers, truthful UI ticks, message state, room open/close, transcript keys, contact trust |
| [STORY.md](STORY.md) | **Human-only** connectivity / discovery story (agents: read, never edit) |
| [TRANSPORT.md](TRANSPORT.md) | libp2p transport stack, discovery, WAN/LAN dial policy, relay, **§ CGNAT / mobile-data relay reservation**, **§ Outbound peer relay dials vs own reservation** |
| [GHAL_BOL_DM_MSG_V1.md](GHAL_BOL_DM_MSG_V1.md) | Wire format, `ack_received` / `ack_read`, upkeep |
| [GHAL_BOL_URI_SCHEME.md](GHAL_BOL_URI_SCHEME.md) | Connect invites: `ghalbol.com`, `ghalbol://` |
| [COORDINATION_SERVER.md](COORDINATION_SERVER.md) | Run/test `ghal_bol_server`, local dev stack, prod `coord.ghalbol.com`, **WAN troubleshooting** |
| [../ghal_bol_server/deploy/README.md](../ghal_bol_server/deploy/README.md) | Dev server + bore/ngrok; **§ Regression prevention** (relay TCP vs coord HTTP) |
| [IDENTITY.md](IDENTITY.md) | Local identity: create/import, export, ownership |
| [PREMIUM_SERVICES.md](PREMIUM_SERVICES.md) | Optional paid Tier 3 relay (separate from messaging keys) |
| [GHAL_BOL_VOICE_V1.md](GHAL_BOL_VOICE_V1.md) | Call signaling + WebRTC media (fallback) |
| [GHAL_BOL_CALL_NATIVE_V2.md](GHAL_BOL_CALL_NATIVE_V2.md) | Native Rust voice engine (shipping) |
| [GHAL_BOL_VIDEO_NATIVE_V1.md](GHAL_BOL_VIDEO_NATIVE_V1.md) | Native Rust video wire/engine (shipping when negotiated) |
| [WEB_SITE.md](WEB_SITE.md) | ghalbol.com static build, `/connect/…`, Linux download |
| [ANDROID_APP_LINKS.md](ANDROID_APP_LINKS.md) | Verified App Links for HTTPS invites |
| [PRODUCTION_RELEASE.md](PRODUCTION_RELEASE.md) | P0/P1/P2 ship checklist |
| [PRIVACY_POLICY.md](PRIVACY_POLICY.md) | Privacy policy draft (Play Console) |
| [PLAY_STORE_LISTING.md](PLAY_STORE_LISTING.md) | Play Store listing and Data safety draft |
| [../README.md](../README.md) | Product vision, networking model, repo map |
| [../AGENTS.md](../AGENTS.md) | AI agent guide and debugging checklist |

**Public site:** `https://ghalbol.com` — **Coordination:** `https://coord.ghalbol.com`

**Invite URLs:** `https://ghalbol.com/connect/<public_key_hex>` and `ghalbol://connect/<public_key_hex>`.
