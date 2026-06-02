import "app_log.dart";
import "ghal_bol_ffi.dart";

/// One persisted chat row (user-visible messages only).
class StoredChatLine {
  const StoredChatLine({
    required this.localId,
    required this.text,
    required this.outgoing,
    this.from,
    this.messageId,
    this.delivery = "pending",
    this.createdAtMs,
    this.readAckSent = false,
  });

  final String localId;
  final String text;
  final bool outgoing;
  final String? from;
  final String? messageId;
  final String delivery;
  final int? createdAtMs;
  final bool readAckSent;

  Map<String, dynamic> toJson() => {
    "local_id": localId,
    "text": text,
    "outgoing": outgoing,
    if (from != null) "from": from,
    if (messageId != null) "message_id": messageId,
    "delivery": delivery,
    if (createdAtMs != null) "created_at_ms": createdAtMs,
    if (readAckSent) "read_ack_sent": true,
  };

  StoredChatLine copyWith({String? delivery, bool? readAckSent}) => StoredChatLine(
    localId: localId,
    text: text,
    outgoing: outgoing,
    from: from,
    messageId: messageId,
    delivery: delivery ?? this.delivery,
    createdAtMs: createdAtMs,
    readAckSent: readAckSent ?? this.readAckSent,
  );

  static StoredChatLine? fromJson(dynamic raw) {
    if (raw is! Map) return null;
    final localId = raw["local_id"]?.toString();
    final text = raw["text"]?.toString();
    if (localId == null || localId.isEmpty || text == null) return null;
    return StoredChatLine(
      localId: localId,
      text: text,
      outgoing: raw["outgoing"] == true,
      from: raw["from"]?.toString(),
      messageId: raw["message_id"]?.toString(),
      delivery: raw["delivery"]?.toString() ?? "pending",
      createdAtMs: raw["created_at_ms"] is int ? raw["created_at_ms"] as int : null,
      readAckSent: raw["read_ack_sent"] == true,
    );
  }
}

/// Local transcript — persisted in **`ghal_bol`** ([`GhalBolFfi.transcriptLoadMerged`]).
class ChatTranscriptStore {
  ChatTranscriptStore._();

  static final Map<String, List<StoredChatLine>> _threadMemoryCache = {};

  static String _threadCacheKey(String appNamespace, Set<String> conversationKeys) {
    final keys = conversationKeys.map((e) => e.trim()).where((e) => e.isNotEmpty).toList()..sort();
    return "${appNamespace.trim()}|${keys.join("|")}";
  }

  static List<StoredChatLine>? peekCachedThread({
    required String appNamespace,
    required Set<String> conversationKeys,
  }) {
    return _threadMemoryCache[_threadCacheKey(appNamespace, conversationKeys)];
  }

  /// Drop cached thread rows after native poll patched delivery on disk (:p2p / daemon).
  static void invalidateThreadCache({
    required String appNamespace,
    Set<String>? conversationKeys,
  }) {
    final ns = appNamespace.trim();
    if (ns.isEmpty) return;
    if (conversationKeys == null || conversationKeys.isEmpty) {
      _threadMemoryCache.removeWhere((k, _) => k.startsWith("$ns|"));
      return;
    }
    final keys = conversationKeys.map((e) => e.trim()).where((e) => e.isNotEmpty).toList();
    if (keys.length == 1) {
      _threadMemoryCache.remove(_threadCacheKey(ns, {keys.first}));
      return;
    }
    _threadMemoryCache.remove(_threadCacheKey(ns, keys.toSet()));
    for (final k in keys) {
      _threadMemoryCache.remove(_threadCacheKey(ns, {k}));
    }
  }

  static Future<String> resolvePath() async => "";

  static Future<void> warmCache({String? appNamespace}) async {
    if (appNamespace == null || appNamespace.trim().isEmpty) return;
    await resolvePathForNamespace(appNamespace.trim());
  }

  /// Preload one thread into [_threadMemoryCache] so chat UI can paint immediately.
  static Future<void> warmThreadCache({
    required String appNamespace,
    String? conversationKey,
    Set<String>? conversationKeys,
  }) async {
    final keys = conversationKeys ??
        (conversationKey != null && conversationKey.trim().isNotEmpty
            ? {conversationKey.trim()}
            : <String>{});
    if (keys.isEmpty) return;
    final canon = conversationKey?.trim() ?? (keys.length == 1 ? keys.first : "");
    await loadMerged(
      appNamespace: appNamespace,
      conversationKeys: keys,
      cacheUnderConversationKey: canon.isNotEmpty ? canon : null,
    );
  }

  static Future<String> resolvePathForNamespace(String appNamespace) async {
    return GhalBolFfi.transcriptResolvePath(appNamespace) ?? "";
  }

  static Future<List<StoredChatLine>> loadMerged({
    required String appNamespace,
    required Set<String> conversationKeys,
    String? matchInboundFromPeerId,
    /// When set, also cache under this single key (for hub warm + peek by canonical conv).
    String? cacheUnderConversationKey,
  }) async {
    final raw = GhalBolFfi.transcriptLoadMerged(appNamespace, {
      "conversation_keys": conversationKeys.toList(),
      if (matchInboundFromPeerId != null && matchInboundFromPeerId.trim().isNotEmpty)
        "match_inbound_from_peer_id": matchInboundFromPeerId.trim(),
    });
    final lines = raw.map(StoredChatLine.fromJson).whereType<StoredChatLine>().toList();
    final cached = List<StoredChatLine>.from(lines);
    _threadMemoryCache[_threadCacheKey(appNamespace, conversationKeys)] = cached;
    final canon = cacheUnderConversationKey?.trim() ?? "";
    if (canon.isNotEmpty) {
      _threadMemoryCache[_threadCacheKey(appNamespace, {canon})] = cached;
    }
    return lines;
  }

  static Future<List<StoredChatLine>> load({
    required String appNamespace,
    required String conversationKey,
  }) =>
      loadMerged(appNamespace: appNamespace, conversationKeys: {conversationKey});

  static Future<void> appendIfNew({
    required String appNamespace,
    required String conversationKey,
    required StoredChatLine line,
  }) async {
    final ok = GhalBolFfi.transcriptAppendIfNew(
      appNamespace,
      conversationKey,
      line.toJson(),
    );
    AppLog.instance.flow(
      "Transcript",
      "append conv=$conversationKey mid=${line.messageId} outgoing=${line.outgoing} ok=$ok",
    );
  }

  static Future<void> patchInboundReadAckSent({
    required String appNamespace,
    required String conversationKey,
    required String messageId,
  }) async {
    GhalBolFfi.transcriptPatchInboundReadAckSent(
      appNamespace,
      conversationKey: conversationKey,
      messageId: messageId,
    );
  }

  static Future<void> patchOutgoingDelivery({
    required String appNamespace,
    required String conversationKey,
    required String messageId,
    required String delivery,
  }) async {
    GhalBolFfi.transcriptPatchOutgoingDelivery(
      appNamespace,
      conversationKey: conversationKey,
      messageId: messageId,
      delivery: delivery,
    );
  }

  static Future<void> repairCorruptOutgoingDeliveryOnce({required String appNamespace}) async {}

  static Future<void> save({
    required String appNamespace,
    required String conversationKey,
    required List<StoredChatLine> lines,
  }) async {
    GhalBolFfi.transcriptSave(
      appNamespace,
      conversationKey,
      lines.map((e) => e.toJson()).toList(),
    );
    invalidateThreadCache(
      appNamespace: appNamespace,
      conversationKeys: {conversationKey},
    );
  }

  static Future<List<StoredChatLine>> trySalvageQuarantinedThread({
    required String appNamespace,
    required String conversationKey,
  }) async =>
      load(appNamespace: appNamespace, conversationKey: conversationKey);
}
