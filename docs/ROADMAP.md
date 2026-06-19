# Product roadmap (human backlog)

Informal product notes — **not** agent implementation specs. Agents must **not** implement from this file unless the user explicitly asks.

| Former `STORY.md` section | Canonical doc now |
|---------------------------|-------------------|
| `# Story` connectivity / WAN / LAN / coord rules | [TRANSPORT.md](TRANSPORT.md) § **Connectivity lifecycle** |
| Coord register “when / when not” | [COORDINATION_SERVER.md](COORDINATION_SERVER.md) § **Client register & heartbeat policy** |
| UI session, acks, ticks | [DESIGN.md](DESIGN.md) |
| Call signaling wire discipline | [DESIGN.md](DESIGN.md) § **Call UI lifecycle and privacy** |
| `# Now` / `# Next` backlog (below) | This file |

## Connectivity goals (product)

- Full network awareness: active interface, internet up/down, global vs LAN reachability.
- Coord registration only when needed (endpoint change, failed register, relay reservation, handover, stale presence) — steady and accurate, not spam; client must never believe it is registered when coord does not list a valid dialable address.
- Fast, stable peer connect; LAN ↔ WAN handover should not leave chat dead for minutes when both peers are online.
- WhatsApp-level confidence that messages and acks actually reach the peer.

## Engineering discipline (human)

- Fix or extend behaviour **without regressing** what already works; understand impact of removing or changing code before shipping.
- Connectivity “don’t flood” means throttle **storms** (repeated `listen_on`, redundant dials) — **not** skipping required relay reservation, coord lookup, or WAN recovery. See [AGENTS.md](../AGENTS.md) connectivity misread table.

## Planned / not shipped

- Change password flow
- Chat history: paginated load on scroll (avoid empty gaps when scroll math misses the fetch trigger; do not load entire transcript at once)
- Attachments (docs, photos, etc.)
- Status feature (WhatsApp-style)
