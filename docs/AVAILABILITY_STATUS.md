# Availability status

**Status:** Implemented.

User-chosen **availability text** on the local roster (presets + custom). This is **not**
transport online/offline and **not** Stories — coord presence stays dial-only.

## Model

| Piece | Owner | Storage |
|-------|--------|---------|
| Local status string | Rust prefs | `preferences_v1.json` → `availability_status` |
| Peer status (received) | Rust contacts | `contacts_v1.json` → `availability_status` |
| Sync | Rust | Native-connect `MsgKind::AvailabilityStatus` **and** delivery-mailbox control upload (same sealed inner) on set |
| UI | Makepad | Set status (Settings → Availability and About); roster subtitle chip |

## Presets (v1)

- `` (clear / none)
- `Available`
- `Busy`
- `Away`
- `In a call`
- Custom free text (sanitized, max 64 chars — same rules as display alias)

## Wire

Envelope kind: `availability_status` (same transport KEM seal as text).

Inner JSON:

```json
{ "status": "Busy", "updated_at_ms": 1735689600000 }
```

- Not shown as a chat bubble; applied to the contact row only.
- No delivery/read ticks (control message).
- On inbound: update `SavedContact.availability_status` + bump contacts change version.

## Sync policy

1. User sets status → save prefs → enqueue sealed status DM (`MsgKind::AvailabilityStatus`) to each registered DM peer over **native-connect**, **and** upload a short-TTL availability control envelope on the **delivery mailbox** for each secp256k1 contact.
2. When a peer session becomes ready → send current local status once on native-connect (so late contacts learn it).
3. Empty status clears the peer’s stored field when received.

Mailbox availability uses the same offline seal as text (`kind: availability_status` inner JSON). Non-secp contacts only receive status over native-connect until multi-algo mailbox seal ships.

## Anti-patterns

- Do not treat status as online/offline presence for dial policy.
- Do not put status in coord register payloads.
- Do not invent status in Makepad without native store.
