import "package:flutter/foundation.dart";

import "app_log.dart";
import "chat_transcript_store.dart";
import "ghal_bol_ffi.dart";
import "identity_display_name.dart";
import "public_key_hex.dart";
import "saved_contact.dart";

/// Contact roster — persisted in **`ghal_bol`** ([`GhalBolFfi.contactsList`]).
class ContactStore {
  ContactStore._();

  static final ValueNotifier<int> changeCount = ValueNotifier<int>(0);
  static final ValueNotifier<int> rosterChangeCount = ValueNotifier<int>(0);
  /// Ack/preview-only poll updates — must not trigger full roster P2P sync.
  static final ValueNotifier<int> previewChangeCount = ValueNotifier<int>(0);

  static void _bumpList() {
    changeCount.value++;
    rosterChangeCount.value++;
  }

  static void _bumpPreview() {
    changeCount.value++;
    previewChangeCount.value++;
  }

  /// Native persisted contacts/transcript — reload hub list (both Android and desktop).
  static void bumpListFromPoll() => changeCount.value++;

  /// Poll applied preview/delivery only (no new roster row).
  static void bumpPreviewFromPoll() => previewChangeCount.value++;

  static Future<List<SavedContact>> listContacts(String appNamespace) async {
    if (!GhalBolFfi.isContactsStoreAvailable) {
      AppLog.instance.w("Contacts", "list: contacts FFI unavailable");
      return [];
    }
    final raw = GhalBolFfi.contactsList(appNamespace);
    final list = raw.map(SavedContact.fromJson).whereType<SavedContact>().toList();
    final path = await ChatTranscriptStore.resolvePathForNamespace(appNamespace);
    AppLog.instance.flow(
      "Contacts",
      "list ns=$appNamespace count=${list.length}${path.isNotEmpty ? " transcript=$path" : ""}",
    );
    return list;
  }

  static Future<SavedContact?> findByPublicKey({
    required String appNamespace,
    required String publicKeyHex,
  }) async {
    final c = GhalBolFfi.contactsFind(appNamespace, {"public_key_hex": publicKeyHex.trim()});
    return c == null ? null : SavedContact.fromJson(c);
  }

  static Future<SavedContact?> findByPeerId({
    required String appNamespace,
    required String libp2pPeerId,
  }) async {
    final c = GhalBolFfi.contactsFind(appNamespace, {"libp2p_peer_id": libp2pPeerId.trim()});
    return c == null ? null : SavedContact.fromJson(c);
  }

  /// Updates only the contact display alias (empty [raw] clears). Other fields unchanged.
  static Future<SavedContact?> updateDisplayAlias({
    required String appNamespace,
    required SavedContact contact,
    required String raw,
  }) async {
    if (!contact.hasPublicKey) return null;
    final body = contact.toJson();
    final alias = ghalSanitizePeerAlias(raw);
    body["display_alias"] = alias ?? "";
    final r = GhalBolFfi.contactsUpsert(appNamespace, body);
    if (r["ok"] != true) {
      AppLog.instance.w("Contacts", "update_display_alias failed: ${r["error"]}");
      return null;
    }
    // Alias-only: do not bump [rosterChangeCount] (avoids P2P re-sync from Contacts screen).
    // Defer so Contacts edit dialog route can finish popping before hub/listeners rebuild.
    Future.microtask(() => changeCount.value++);
    final c = r["contact"];
    if (c is Map) {
      return SavedContact.fromJson(c);
    }
    return findByPublicKey(
      appNamespace: appNamespace,
      publicKeyHex: contact.publicKeyHex,
    );
  }

  static Future<SavedContact> upsertContact({
    required String appNamespace,
    required SavedContact contact,
  }) async {
    final r = GhalBolFfi.contactsUpsert(appNamespace, contact.toJson());
    if (r["ok"] != true) {
      AppLog.instance.w("Contacts", "upsert failed: ${r["error"]}");
    } else {
      AppLog.instance.flowJson("Contacts", "upsert", {
        "pk": contact.publicKeyHex,
        "has_pk": contact.hasPublicKey,
      });
    }
    _bumpList();
    final c = r["contact"];
    if (c is Map) {
      return SavedContact.fromJson(c) ?? contact;
    }
    return contact;
  }

  static Future<void> removeContact({
    required String appNamespace,
    required SavedContact contact,
  }) async {
    if (GhalBolFfi.contactsRemove(appNamespace, contact.toJson())) {
      _bumpList();
    }
  }

  static Future<void> deleteContact({
    required String appNamespace,
    required String publicKeyHex,
  }) async {
    final pk = publicKeyHex.trim();
    if (!isValidPublicKeyHex(pk)) return;
    final existing = await findByPublicKey(appNamespace: appNamespace, publicKeyHex: pk);
    if (existing != null) {
      await removeContact(appNamespace: appNamespace, contact: existing);
    }
  }

  static Future<void> touchChatPreview({
    required String appNamespace,
    required String contactPublicKeyHex,
    required String preview,
    bool markUnread = false,
    int? messageAtMs,
  }) async {
    GhalBolFfi.contactsRecordInboundPreview(appNamespace, {
      "sender_public_key_hex": contactPublicKeyHex.trim(),
      "preview": preview,
      "mark_unread": markUnread,
      ...?(messageAtMs == null ? null : {"message_at_ms": messageAtMs}),
    });
    _bumpPreview();
  }

  static Future<void> mergeDiscoveredPeerId({
    required String appNamespace,
    required String publicKeyHex,
    String? libp2pPeerId,
  }) =>
      mergeDiscoveredContact(
        appNamespace: appNamespace,
        publicKeyHex: publicKeyHex,
      );

  static Future<void> mergeDiscoveredContact({
    required String appNamespace,
    required String publicKeyHex,
  }) async {
    if (GhalBolFfi.contactsMergeDiscovered(appNamespace, publicKeyHex, "")) {
      _bumpPreview();
    }
  }

  static Future<void> recordInboundPreview({
    required String appNamespace,
    required String senderPublicKeyHex,
    required String preview,
    required bool markUnread,
    int? messageAtMs,
  }) async {
    await touchChatPreview(
      appNamespace: appNamespace,
      contactPublicKeyHex: senderPublicKeyHex,
      preview: preview,
      markUnread: markUnread,
      messageAtMs: messageAtMs,
    );
  }

  static Future<void> reconcilePreviewsFromTranscript(String appNamespace) async {
    // Native P2P + transcript are source of truth; hub reloads from [listContacts].
  }

  static Future<void> clearUnread({
    required String appNamespace,
    required String publicKeyHex,
  }) async {
    if (GhalBolFfi.contactsClearUnread(appNamespace, publicKeyHex)) {
      _bumpPreview();
    }
  }

  static Future<void> clearUnreadForContact({
    required String appNamespace,
    required SavedContact contact,
  }) async {
    if (contact.hasPublicKey) {
      await clearUnread(appNamespace: appNamespace, publicKeyHex: contact.publicKeyHex);
    }
  }

  /// Add / Block / first-send — updates `is_known` / `is_blocked` on the contact row.
  static Future<SavedContact?> setTrust({
    required String appNamespace,
    required String publicKeyHex,
    bool? isKnown,
    bool? isBlocked,
  }) async {
    final pk = publicKeyHex.trim().toLowerCase();
    if (!isValidPublicKeyHex(pk)) return null;
    if (isKnown == null && isBlocked == null) return null;
    final body = <String, dynamic>{"public_key_hex": pk};
    if (isKnown != null) body["is_known"] = isKnown;
    if (isBlocked != null) body["is_blocked"] = isBlocked;
    final r = GhalBolFfi.contactsSetTrust(appNamespace, body);
    if (r["ok"] != true) {
      AppLog.instance.w("Contacts", "set_trust failed: ${r["error"]}");
      return null;
    }
    _bumpList();
    final c = r["contact"];
    if (c is Map) {
      return SavedContact.fromJson(c);
    }
    return findByPublicKey(appNamespace: appNamespace, publicKeyHex: pk);
  }

  @visibleForTesting
  static void resetForTest() {}
}
