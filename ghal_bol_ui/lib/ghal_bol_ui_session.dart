import "package:ghal_bol_ui/p2p_event_bridge.dart";
import "package:ghal_bol_ui/public_key_hex.dart";

/// **Only** UI→native session signals the integrator may send.
///
/// All messaging policy (acks, delivery, dial, outbox) lives in `ghal_bol`.
/// Call these when layout or app lifecycle changes; never set ack/foreground RPCs directly.
abstract final class GhalBolUiSession {
  /// App is interactive (`resumed`). `false` for inactive / paused / background.
  static void setVisible(bool visible) {
    P2pEventBridge.instance.setUiVisible(visible);
  }

  /// Open conversation room (`public_key_hex`), or `null` when no room is on screen.
  static void setRoom(String? publicKeyHex) {
    final p = publicKeyHex?.trim().toLowerCase() ?? "";
    P2pEventBridge.instance.setForegroundConversation(
      isValidPublicKeyHex(p) ? p : null,
    );
  }

  /// Close room and disable read receipts (same as [setRoom](null) + not visible).
  static Future<void> closeRoom() async {
    setRoom(null);
    await awaitApplied();
  }

  /// Wait until the latest session snapshot reached native (`p2p_sync_ui_session`).
  static Future<void> awaitApplied() =>
      P2pEventBridge.instance.awaitUiSessionApplied();
}
