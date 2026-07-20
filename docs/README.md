# Documentation

Canonical index — read [DESIGN.md](DESIGN.md) before changing P2P, acks, invites, or persistence.

**Connectivity policy** (formerly in `STORY.md`, removed): [TRANSPORT.md](TRANSPORT.md) § **Connectivity lifecycle**, § **Network truth**, § **Asymmetric LAN↔WAN mux recovery**; [COORDINATION_SERVER.md](COORDINATION_SERVER.md) § **Client register & heartbeat policy**; [DESIGN.md](DESIGN.md) (acks/UI session). Human product backlog only: [ROADMAP.md](ROADMAP.md).

| Document | Contents |
|----------|----------|
| [DESIGN.md](DESIGN.md) | **Canonical** layers, truthful UI ticks, message state, room open/close, transcript keys, contact trust |
| [TRANSPORT.md](TRANSPORT.md) | native transport, **Connectivity lifecycle**, **Network truth**, **Asymmetric mux recovery**, parallel LAN+WAN, relay/CGNAT |
| [ROADMAP.md](ROADMAP.md) | Human product backlog (not agent specs) |
| [GHAL_BOL_DM_MSG_V1.md](GHAL_BOL_DM_MSG_V1.md) | Wire format, `ack_received` / `ack_read`, upkeep |
| [GHAL_BOL_URI_SCHEME.md](GHAL_BOL_URI_SCHEME.md) | Connect invites: `ghalbol.com`, `ghalbol://` |
| [COORDINATION_SERVER.md](COORDINATION_SERVER.md) | Run/test `ghal_bol_coord`, local dev stack, prod `coord.ghalbol.com`, **WAN troubleshooting** |
| [GHAL_BOL_DELIVERY.md](GHAL_BOL_DELIVERY.md) | Delivery server design (WAN text mailbox); crate `ghal_bol_delivery/` |
| [GHAL_BOL_CONNECT_V1.md](GHAL_BOL_CONNECT_V1.md) | Native connect transport (mDNS + Noise + channel mux + coord bridge) — replaces native connect |