# Documentation

Canonical index — read [DESIGN.md](DESIGN.md) before changing P2P, acks, invites, or persistence.

Connectivity: [TRANSPORT.md](TRANSPORT.md) § **Connectivity lifecycle**, § **Network truth**, § **Asymmetric LAN↔WAN mux recovery**. Coord: [COORDINATION_SERVER.md](COORDINATION_SERVER.md). UI shell: [MAKEPAD_UI.md](MAKEPAD_UI.md).

| Document | Contents |
|----------|----------|
| [DESIGN.md](DESIGN.md) | **Canonical** layers, truthful ticks, room open/close, transcripts, trust, process |
| [MAKEPAD_UI.md](MAKEPAD_UI.md) | Makepad 2 shell (`ghal_bol_app`); Splash, scrolling, language/fonts, `host` |
| [TRANSPORT.md](TRANSPORT.md) | Native connect: lifecycle, network truth, parallel LAN+WAN, bridge/CGNAT, caching |
| [GHAL_BOL_CONNECT_V1.md](GHAL_BOL_CONNECT_V1.md) | Native connect wire (mDNS + Noise + mux + coord bridge) |
| [GHAL_BOL_DM_MSG_V1.md](GHAL_BOL_DM_MSG_V1.md) | DM wire, `ack_received` / `ack_read`, upkeep |
| [GHAL_BOL_URI_SCHEME.md](GHAL_BOL_URI_SCHEME.md) | Connect invites: `ghalbol.com`, `ghalbol://` |
| [GHAL_BOL_DELIVERY.md](GHAL_BOL_DELIVERY.md) | WAN text mailbox design (`ghal_bol_delivery`) |
| [GHAL_BOL_DELIVERY_WIRE_V1.md](GHAL_BOL_DELIVERY_WIRE_V1.md) | Delivery HTTP/WebSocket wire |
| [COORDINATION_SERVER.md](COORDINATION_SERVER.md) | Run/test `ghal_bol_coord`, local/prod, troubleshooting |
| [IDENTITY.md](IDENTITY.md) | Local identity (secp256k1 today) |
| [MULTI_ALGO.md](MULTI_ALGO.md) | Multi-algorithm identity wire |
| [GHAL_BOL_VOICE_V1.md](GHAL_BOL_VOICE_V1.md) | Call signaling (`ghal_bol_call_v1`) |
| [GHAL_BOL_CALL_NATIVE_V2.md](GHAL_BOL_CALL_NATIVE_V2.md) | Native voice engine (shipping) |
| [GHAL_BOL_VIDEO_NATIVE_V1.md](GHAL_BOL_VIDEO_NATIVE_V1.md) | Native video engine (shipping) |
| [VOICE_MESSAGES_PLAN.md](VOICE_MESSAGES_PLAN.md) | Async voice notes over DM |
| [ATTACHMENTS_PLAN.md](ATTACHMENTS_PLAN.md) | File attachments |
| [status.md](status.md) | Availability status |
| [chat_history_pagination.md](chat_history_pagination.md) | Transcript windowing |
| [change_password_flow.md](change_password_flow.md) | Password change |
| [PRODUCTION_RELEASE.md](PRODUCTION_RELEASE.md) | Release / Play / desktop packaging |
| [WEB_SITE.md](WEB_SITE.md) | ghalbol.com hosting |
| [ANDROID_APP_LINKS.md](ANDROID_APP_LINKS.md) | App Links verification |
| [PLAY_STORE_LISTING.md](PLAY_STORE_LISTING.md) | Store copy |
| [PRIVACY_POLICY.md](PRIVACY_POLICY.md) | Privacy policy |
| [PREMIUM_SERVICES.md](PREMIUM_SERVICES.md) | Paid tiers (relay/backup) |
| [env/README.md](../env/README.md) | Coord/delivery env files |
