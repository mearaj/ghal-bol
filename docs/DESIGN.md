# Ghal Bol — system design

How the app works today. Wire detail is in [GHALBOL_DM_MSG_V1.md](GHALBOL_DM_MSG_V1.md). Invites are in [GHALBOL_URI_SCHEME.md](GHALBOL_URI_SCHEME.md). Connectivity is in [TRANSPORT.md](TRANSPORT.md).

## Goals

- End-to-end encrypted **text messaging** via the delivery mailbox ([GHALBOL_DELIVERY.md](GHALBOL_DELIVERY.md)). Text is not a native-connect / LAN P2P product path.
- **Voice and video calls** use native connect: direct on LAN when possible, otherwise through the coord byte bridge.
- No server-side chat history. Each device keeps its own transcript. The delivery server stores ciphertext only until the recipient acknowledges it.
- The recipient decides delivery and read. The sender does not invent ticks.
- One process. `ghalbol_app` calls `ghalbol_core::host`. Debug data is `~/.local/share/com.ghalbol.debug/`. Release data is `~/.local/share/com.ghalbol/`.

## Layers

```text
ghalbol_app (Makepad 2)
  identity, hub, chat, invites, calls
  host::set_open_room, host::poll_event
        │ in-process
ghalbol_core
  keystore, contacts, transcript, delivery, native connect, outbox, acks
```

| Concern | Owner |
|---------|--------|
| Listen, dial, calls, call signaling | `ghalbol_core` (`connect/`) |
| Text mailbox send/recv and acks | `delivery_runtime.rs` |
| Apply events to disk | `dm_event_handler.rs` on poll |
| Contacts, unread, preview, trust | `contacts_v1.rs` |
| Transcript lines and `delivery` | `dm_transcript_store.rs` |
| Invites | `connect_invite_v1.rs` |
| Screens | `ghalbol_app` via `host` |

The UI does not implement ack policy, the outbox, or transcript merge. It does not call coord HTTP.

## End-to-end encryption

Peer traffic uses the device identity and the contact’s identity wire.

| Channel | Mechanism |
|---------|-----------|
| Text messaging | `ghalbol_delivery` ciphertext (`delivery_msg_v1`) |
| Call signaling | `ghalbol_call_v1` with transport KEM on native connect |
| Call audio and video | Keys from the transport secret, AES-GCM per frame |

## Who scans whom

The guest scans the host QR and stores the host public key. The host may have zero contacts until the first inbound message. Both sides do not need each other’s key from a QR.

## Truthful ticks

The UI shows `delivery` only after the transcript on disk says so.

- Tick authority is the **delivery** path ([GHALBOL_DELIVERY.md](GHALBOL_DELIVERY.md)). The sender never invents ticks.
- Poll applies events for display. Poll does not send acks.
- Outbound `pending` while the peer already has the text is normal. Devices do not sync transcripts.
- State only moves forward: `pending` → `sent` → `delivered` → `read`. `read` never downgrades to `delivered`.

| Text (`GHALBOL_DELIVERY_URL` set) | Transcript | Meaning |
|-------------------------------------|------------|---------|
| No tick | `pending` | Not confirmed on the server |
| ✓ | `sent` | Server accepted the upload |
| ✓✓ | `delivered` | Recipient got it |
| ✓✓ in blue | `read` | Recipient had the room open |

Product chat uses this delivery tick model only.

## Room open, leave, and read acks

**Open** means the hub has called `host::set_open_room(Some(peer))` and the chat column is showing. A highlighted roster row is not open.

Product text acks travel on the **delivery** path ([GHALBOL_DELIVERY.md](GHALBOL_DELIVERY.md)):

- Delivery ack is always sent for accepted inbound text.
- Read ack is sent for new inbound text only while that room is open and the app is showing it.
- On leave, `host::set_open_room(None)` freezes `chat_room_exit_at_ms` and drains pending read acks for messages received while the room was open. New mail after leave gets delivery ack only.
- Linux: do not treat a brief focus loss as leaving the room. Android: when the app is not visible, do not send new read acks for the open room.

## Transcripts

One store, `chat_transcript_v1.json`, for LAN and WAN. Load a thread by peer id and public key together so the chat is not empty while the roster preview exists. The UI reloads. It does not append.

## Contact trust

`is_known` and `is_blocked` live on the contact row. They are not on the wire.

- A scanned or manually added contact starts `is_known: true`.
- A row created by the first inbound message starts `is_known: false`.
- Unknown peers still receive messages and still get `ack_received`.
- Add, or the first outbound send, sets `is_known: true`. Block sets `is_blocked: true`.
- Trust does not change foreground order or ack policy.

## Calls

`host::start_voice_call`, `host::start_video_call`, `host::accept_incoming_call`, `host::set_mic_muted`, `host::end_call`. While video is connected, `host::call_picture_pngs` is the local and remote picture. Ending a call stops media. Log out stops the node, which ends an active call.

## Notifications

New inbound text, voice notes and files post a notification when they bump unread — that is, the message is new (not a poll replay) and the chat is not open with read receipts on. One notification per chat, replaced by newer mail and removed by `host::set_open_room(Some(peer))`. The body says what arrived ("New message", "Voice message", "File"), never the message text. Blocked contacts never notify. Code: `ghalbol_core/src/message_notify.rs`. Incoming calls keep their own notification (`incoming_call_notify.rs`).

## Invites

Format 2 carries `public_key_hex` only. Encode and decode live in `connect_invite_v1.rs`. The desktop app registers `ghalbol://`. A link passed on the command line is joined after unlock.

## Persistence

| File | Contents |
|------|----------|
| Keystore | Encrypted identity |
| `contacts_v1.json` | Roster, alias, preview, unread, trust, `chat_room_exit_at_ms` |
| `chat_transcript_v1.json` | Lines, `delivery`, `received_at_ms`, `read_ack_sent` |
| `preferences_v1.json` | Locale, availability, coord URL |

Contacts, the transcript, voice notes and attachments live in the `ghalbol/` subfolder of the data directory. Builds from before the `ghal_bol` → `ghalbol` rename wrote `ghal_bol/` and sealed the keystore with a `ghal_bol.keystore.v1` AAD; `app_paths.rs` moves the old folder on first use (leaving a `ghal_bol` symlink on Unix so stored file paths still open) and `keystore_v1.rs` still unlocks the old AAD. Peers and servers from before the rename do **not** interoperate with new builds (handshake, KDF and signing domains changed) — redeploy coord and delivery from the same commit as the apps.

Do not cache live endpoints (coord presence, mDNS port, bridge tokens) on disk.

## Process

`host::unlock` installs the identity. `host::start_network` starts native connect. `host::poll_event` lets the UI refresh. Lock hides the hub and keeps the node running. Log out calls `host::lock`, which stops the node.

On Linux, launch writes `~/.config/autostart/com.ghalbol.desktop` (debug: `com.ghalbol.debug.desktop`) so the app starts at login. If a keystore exists and the session is locked, the app asks for the password. A brief focus change does not turn read acks off. On Android, `host::set_app_visible(false)` runs when the app goes to the background: the open room stays, and new read acks wait until it is visible again.

## Code map

| Area | Path |
|------|------|
| UI | `apps/ghalbol/src/main.rs` |
| UI API | `ghalbol_core/src/host.rs` |
| Connect and acks | `ghalbol_core/src/connect/` |
| Events to disk | `ghalbol_core/src/dm_event_handler.rs` |
| Contacts | `ghalbol_core/src/contacts_v1.rs` |
| Transcript | `ghalbol_core/src/dm_transcript_store.rs` |
| Delivery | `ghalbol_core/src/delivery_runtime.rs` |
