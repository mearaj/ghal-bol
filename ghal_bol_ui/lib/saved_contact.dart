import "package:ghal_bol_ui/ghal_bol_ffi.dart";

import "ghalbol_connect_invite.dart";
import "identity_display_name.dart";
import "public_key_hex.dart";

/// A peer you can message, keyed by **identity wire** (`[algo:]hex` per MULTI_ALGO.md).
class SavedContact {
  const SavedContact({
    this.publicKeyHex = "",
    this.displayAlias,
    this.lastMessagePreview,
    this.lastMessageAtMs,
    this.unreadCount = 0,
    this.createdAtMs,
    this.updatedAtMs,
    this.isKnown = true,
    this.isBlocked = false,
    this.chatRoomExitAtMs,
  });

  final String publicKeyHex;
  final String? displayAlias;
  final String? lastMessagePreview;
  final int? lastMessageAtMs;
  final int unreadCount;
  final int? createdAtMs;
  final int? updatedAtMs;
  final bool isKnown;
  final bool isBlocked;
  /// Last active in-room moment with this peer (`:p2p` writes; UI read-only).
  final int? chatRoomExitAtMs;

  bool get hasPublicKey => isValidPublicKeyHex(publicKeyHex);

  /// Hub shows **Unknown** while the user has not accepted this peer.
  bool get showUnknownChip => hasPublicKey && !isKnown && !isBlocked;

  /// Room shows Add / Block banner for the same state.
  bool get showTrustBanner => showUnknownChip;

  /// Whether native can encrypt/sign DMs to this peer.
  bool get hasFullKeys => hasPublicKey;

  String get conversationKey => publicKeyHex.trim().toLowerCase();

  /// Transcript thread keys (pk + legacy libp2p PeerId bucket on disk).
  Set<String> get allConversationKeys {
    final keys = <String>{};
    if (hasPublicKey) {
      keys.add(conversationKey);
      final legacyPid = GhalBolFfi.peerIdFromPublicKeyHex(publicKeyHex);
      if (legacyPid != null && legacyPid.isNotEmpty && legacyPid != conversationKey) {
        keys.add(legacyPid);
      }
    }
    keys.removeWhere((k) => k.isEmpty);
    return keys;
  }

  SavedContact copyWith({
    String? publicKeyHex,
    String? displayAlias,
    String? lastMessagePreview,
    int? lastMessageAtMs,
    int? unreadCount,
    int? updatedAtMs,
    bool? isKnown,
    bool? isBlocked,
    bool clearLastMessage = false,
  }) {
    return SavedContact(
      publicKeyHex: publicKeyHex ?? this.publicKeyHex,
      displayAlias: displayAlias ?? this.displayAlias,
      lastMessagePreview: clearLastMessage ? null : (lastMessagePreview ?? this.lastMessagePreview),
      lastMessageAtMs: clearLastMessage ? null : (lastMessageAtMs ?? this.lastMessageAtMs),
      unreadCount: unreadCount ?? this.unreadCount,
      createdAtMs: createdAtMs,
      updatedAtMs: updatedAtMs ?? this.updatedAtMs,
      isKnown: isKnown ?? this.isKnown,
      isBlocked: isBlocked ?? this.isBlocked,
    );
  }

  Map<String, dynamic> toJson() => {
    if (publicKeyHex.isNotEmpty) "public_key_hex": publicKeyHex,
    if (displayAlias != null && displayAlias!.isNotEmpty) "display_alias": displayAlias,
    if (lastMessagePreview != null) "last_message_preview": lastMessagePreview,
    if (lastMessageAtMs != null) "last_message_at_ms": lastMessageAtMs,
    "unread_count": unreadCount,
    "is_known": isKnown,
    "is_blocked": isBlocked,
    if (createdAtMs != null) "created_at_ms": createdAtMs,
    if (updatedAtMs != null) "updated_at_ms": updatedAtMs,
    if (chatRoomExitAtMs != null) "chat_room_exit_at_ms": chatRoomExitAtMs,
  };

  static SavedContact? fromJson(dynamic raw) {
    if (raw is! Map) return null;
    var pk = raw["public_key_hex"]?.toString().trim().toLowerCase() ?? "";
    if (!isValidPublicKeyHex(pk)) {
      final legacyPid = raw["libp2p_peer_id"]?.toString().trim() ?? "";
      if (legacyPid.isNotEmpty) {
        pk = GhalBolFfi.publicKeyHexFromPeerId(legacyPid)?.trim().toLowerCase() ?? "";
      }
    }
    if (!isValidPublicKeyHex(pk)) return null;
    return SavedContact(
      publicKeyHex: pk,
      displayAlias: raw["display_alias"]?.toString(),
      lastMessagePreview: raw["last_message_preview"]?.toString(),
      lastMessageAtMs: raw["last_message_at_ms"] is int ? raw["last_message_at_ms"] as int : null,
      unreadCount: raw["unread_count"] is int ? (raw["unread_count"] as int).clamp(0, 9999) : 0,
      isKnown: raw.containsKey("is_known") ? raw["is_known"] == true : true,
      isBlocked: raw["is_blocked"] == true,
      createdAtMs: raw["created_at_ms"] is int ? raw["created_at_ms"] as int : null,
      updatedAtMs: raw["updated_at_ms"] is int ? raw["updated_at_ms"] as int : null,
      chatRoomExitAtMs: raw["chat_room_exit_at_ms"] is int ? raw["chat_room_exit_at_ms"] as int : null,
    );
  }

  static SavedContact fromInvite(GhalBolConnectInvite inv, {int? nowMs}) {
    final t = nowMs ?? DateTime.now().millisecondsSinceEpoch;
    final pkRaw = inv.publicKeyHex.trim().toLowerCase();
    if (!isValidPublicKeyHex(pkRaw)) {
      throw ArgumentError("invite requires public_key_hex");
    }
    return SavedContact(
      publicKeyHex: pkRaw,
      displayAlias: ghalSanitizePeerAlias(inv.peerAlias),
      isKnown: true,
      isBlocked: false,
      createdAtMs: t,
      updatedAtMs: t,
    );
  }

  GhalBolConnectInvite? toConnectInvite() {
    final pk = publicKeyHex.trim().toLowerCase();
    if (!isValidPublicKeyHex(pk)) return null;
    return GhalBolConnectInvite(
      topic: kDefaultGossipTopic,
      publicKeyHex: pk,
      peerAlias: displayAlias,
    );
  }
}
