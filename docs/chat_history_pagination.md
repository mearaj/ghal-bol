# Chat History Pagination

**Status:** Implemented (growing-window model).

Long transcripts are not loaded or rendered all at once. The chat screen loads
the **newest N** lines and grows the window by one page when the user scrolls up
to older history.

## Model

- The native store returns the newest `limit` lines plus a `has_more` flag
  ([`thread_view_limited`](../ghal_bol_core/src/dm_transcript_store.rs)).
- `limit` omitted → full thread (`has_more = false`) — used by hub warm/cache.
- The Flutter chat screen keeps a `_transcriptWindowLimit` (default 50). On
  scroll toward the top of the reversed list, it grows the limit by
  `_transcriptPageSize` and reloads ([`chat_screen.dart`](../ghal_bol_ui/lib/chat_screen.dart)
  `_onScrollForOlderPage` / `_loadOlderMessages`).
- Because the list uses `reverse: true`, appended older lines render above the
  current view without jumping the scroll position — this avoids the empty-gap
  problem when scroll math misses the fetch trigger.

## Why grow-the-window (not before_ms cursor)

The existing chat paint model applies a full, revision-gated snapshot of the
loaded window and owns the truthful delivery/read tick guards (DESIGN.md
§ "Transcript UI view contract"). Growing the window keeps that single-snapshot
model intact: every refresh is still a consistent newest-N view, so live tick
updates on poll continue to work with no separate prepend/merge-by-id path.

## Ownership

- Rust owns the slice + `has_more` (`ghal_bol_core`).
- FFI/RPC: `transcript_load_merged` accepts optional `limit`, returns `has_more`
  (same method; optional fields only — daemon SDK parity preserved).
- Flutter is UI only: window size, scroll trigger, repaint.

## Wire

Request (existing method, added optional field):

```json
{ "app_namespace": "...", "conversation_keys": ["..."], "limit": 50 }
```

Response adds `has_more`:

```json
{ "ok": true, "revision": 12, "has_more": true, "lines": [ /* newest 50 */ ] }
```
