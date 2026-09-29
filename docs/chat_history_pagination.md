# Chat History Pagination

**Status:** Implemented (growing-window model).

Long transcripts are not loaded or rendered all at once. The chat screen loads
the **newest N** lines and grows the window by one page when the user asks for
older history.

## Model

- The native store returns the newest `limit` lines plus a `has_more` flag
  ([`thread_view_limited`](../ghal_bol_core/src/dm_transcript_store.rs)).
- `limit` omitted → full thread (`has_more = false`) — used by hub warm/cache.
- The Makepad chat screen keeps a `window_limit` (default 50 via `transcript_limit()`).
  **Older messages** grows the limit by 50 and reloads via `host::load_transcript`.
- The chat list paints the loaded window as a newest-first snapshot so growing the
  window does not invent a separate prepend/merge path.

## Why grow-the-window (not before_ms cursor)

The chat paint model applies a full snapshot of the loaded window and owns the
truthful delivery/read tick guards ([DESIGN.md](DESIGN.md)). Growing the window
keeps that single-snapshot model intact: every refresh is still a consistent
newest-N view, so live tick updates on poll continue to work.

## Ownership

- Rust owns the slice + `has_more` (`ghal_bol_core`).
- The UI calls `host::load_transcript(namespace, peer, limit)`.
- Makepad is UI only: window size, the older-messages control, and repaint.

## Wire

Request (existing method, added optional field):

```json
{ "app_namespace": "...", "conversation_keys": ["..."], "limit": 50 }
```

Response adds `has_more`:

```json
{ "ok": true, "revision": 12, "has_more": true, "lines": [ /* newest 50 */ ] }
```
