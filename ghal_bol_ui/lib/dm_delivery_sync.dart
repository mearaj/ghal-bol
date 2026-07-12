/// DM delivery sync — policy comments only (ticks come from native transcript fields).
///
/// **Truthful UI:** show ticks only after native patches transcript on poll / FFI reload.
/// Never show delivered/read because the user opened the chat or because send succeeded.
///
/// Intent: recipient decides delivery/read; sender and recipient views may differ; no cross-device
/// tick sync. Hub / lifecycle: [GhalBolUiSession] only — native owns ack policy.
/// See `docs/DESIGN.md` § “Truthful status in the UI”, “Delivery mode — outbound ticks”.
///
/// Do not implement ack send or outbox logic in Dart.
///
/// ### Delivery mode (`GHAL_BOL_DELIVERY_URL` set) — chat text
///
/// Outbound ticks (sender): clock → single black → double black → double blue.
///
/// | UI | Transcript `delivery` | Native trigger |
/// |----|----------------------|----------------|
/// | Clock | `pending` | Local save; upload not confirmed |
/// | Single black ✓ | `sent` | Server `message.upload.ok` |
/// | Double black ✓✓ | `delivered` | `message.ack_to_sender` (recipient `inbox.ack`) |
/// | Double blue ✓✓ | `read` | `message.read_to_sender` (recipient `inbox.read`) |
///
/// See `docs/GHAL_BOL_DELIVERY.md` § “Outbound ticks”.
///
/// ### P2P libp2p DM (LAN text + calls only when `GHAL_BOL_DELIVERY_URL` set)
///
/// #### Peer A (sent message `X`)
/// - **Outbound** `X`: `pending` until **recipient B** sends `ack_received` → `delivered`;
///   `ack_read` → `read`. A never upgrades ticks without an ack from B.
/// - Native **outbox** on A resends `X` until B's `ack_received` (efficiency; not a UI tick).
///
/// #### Peer B (received message `X`)
/// - **Inbound** `X`: no delivery ticks (B is not the authority on A's send state).
/// - **Always** (including `:p2p` / daemon, UI optional): native sends **`ack_received`** for `X`
///   (`chat_server.rs` — not from this Dart file). Retries on ~1s upkeep until the stream accepts.
/// - **Inside the open chat room** (hub set foreground): native **also** sends **`ack_read`** for `X`.
/// - If `X` arrived after B left the room, **no** new `ack_read` until room opens again.
/// - If `X` arrived while the room was open, native **keeps retrying** `ack_read` after leave
///   until A confirms — leaving does not cancel that backlog.
/// - Opening the room again seeds transcript rows still missing `read_ack_sent`.
/// - A's native stack **resends the text** for `X` on ~1s upkeep until B's `ack_received` or
///   `ack_read` arrives. B retries acks from native queues if the stream was down — **no Flutter poll required to send acks**.
/// - **Inbound** `readAckSent`: true only after **sender A** echoes `ack_received` confirming B's
///   `ack_read` (`ref_id == X`). Never set just because B opened the chat.
///
/// There is no shared global state and no pull-sync of the other side's ticks — only ack frames
/// on the wire. Flutter persists each peer's local view in [ChatTranscriptStore].
///
/// UI rule: outbound ticks only from native transcript `delivery` after poll/FFI merge — never from
/// send-queue success alone (delivery `sent` requires server `message.upload.ok`).

library;

/// Recipient told us they got our outbound text (`ack_received` on our message id).
const String kRecipientAckDelivered = "ack_received";

/// Recipient told us they read our outbound text (`ack_read` on our message id).
const String kRecipientAckRead = "ack_read";

/// Sender confirmed they got our `ack_read` for their inbound text (`ack_received`, inbound id).
const String kSenderConfirmedReadReceipt = "ack_received";

bool isRecipientOutboundAckKind(String? msgKind) {
  final k = msgKind?.trim() ?? "";
  return k == kRecipientAckDelivered || k == kRecipientAckRead;
}

String outboundDeliveryForRecipientAck(String msgKind) =>
    msgKind.trim() == kRecipientAckRead ? "read" : "delivered";
