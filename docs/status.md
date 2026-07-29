# Availability status

**Status:** Implemented.

User-chosen **availability text** on the local roster (presets + custom). This is **not**
transport online/offline and **not** Stories — coord presence stays dial-only.

## Model

| Piece | Owner | Storage |
|-------|--------|---------|
| Local status string | Rust prefs | `preferences_v1.json` → `availability_status` |
| Peer status (received) | Rust contacts | `contacts_v1.json` → `availability_status` |
| Sync | Rust | Sealed DM `MsgKind::AvailabilityStatus` on set + on connect |
| UI | Flutter | Set status (Identity / More); roster subtitle chip |

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

1. User sets status → save prefs → enqueue sealed status DM to each registered DM peer.
2. When a peer session becomes ready → send current local status once (so late contacts learn it).
3. Empty status clears the peer’s stored field when received.

## Anti-patterns

- Do not treat status as online/offline presence for dial policy.
- Do not put status in coord register payloads.
- Do not invent status in Flutter without native store.
