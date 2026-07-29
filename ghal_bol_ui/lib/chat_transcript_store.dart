import "ghal_bol_ffi.dart";
import "ghal_bol_p2p.dart";

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
    this.receivedAtMs,
    this.msgKind = "text",
    this.durationMs,
    this.audioPath,
    this.fileName,
    this.mimeType,
    this.sizeBytes,
    this.localPath,
  });

  final String localId;
  final String text;
  final bool outgoing;
  final String? from;
  final String? messageId;
  final String delivery;
  final int? createdAtMs;
  final bool readAckSent;
  final int? receivedAtMs;
  final String msgKind;
  final int? durationMs;
  final String? audioPath;
  final String? fileName;
  final String? mimeType;
  final int? sizeBytes;
  final String? localPath;

  Map<String, dynamic> toJson() => {
    "local_id": localId,
    "text": text,
    "outgoing": outgoing,
    if (from != null) "from": from,
    if (messageId != null) "message_id": messageId,
    "delivery": delivery,
    if (createdAtMs != null) "created_at_ms": createdAtMs,
    if (receivedAtMs != null) "received_at_ms": receivedAtMs,
    if (readAckSent) "read_ack_sent": true,
    if (msgKind != "text") "msg_kind": msgKind,
    if (durationMs != null) "duration_ms": durationMs,
    if (audioPath != null) "audio_path": audioPath,
    if (fileName != null) "file_name": fileName,
    if (mimeType != null) "mime_type": mimeType,
    if (sizeBytes != null) "size_bytes": sizeBytes,
    if (localPath != null) "local_path": localPath,
  };

  StoredChatLine copyWith({String? delivery, bool? readAckSent}) =>
      StoredChatLine(
        localId: localId,
        text: text,
        outgoing: outgoing,
        from: from,
        messageId: messageId,
        delivery: delivery ?? this.delivery,
        createdAtMs: createdAtMs,
        readAckSent: readAckSent ?? this.readAckSent,
        receivedAtMs: receivedAtMs,
        msgKind: msgKind,
        durationMs: durationMs,
        audioPath: audioPath,
        fileName: fileName,
        mimeType: mimeType,
        sizeBytes: sizeBytes,
        localPath: localPath,
      );

  static StoredChatLine? fromJson(dynamic raw) {
    if (raw is! Map) return null;
    final localId = raw["local_id"]?.toString();
    final text = raw["text"]?.toString() ?? "";
    final fileNameRaw = raw["file_name"]?.toString().trim();
    final fileName = (fileNameRaw != null && fileNameRaw.isNotEmpty)
        ? fileNameRaw
        : _fileNameFromAttachmentPreview(text);
    final msgKind = raw["msg_kind"]?.toString().trim();
    var kind = msgKind == null || msgKind.isEmpty ? "text" : msgKind;
    // Legacy rows sometimes kept only "📎 name" with msg_kind stripped.
    if (kind == "text" &&
        (fileName != null || text.trimLeft().startsWith("📎"))) {
      kind = "attachment_offer";
    }
    if (localId == null || localId.isEmpty) return null;
    if (kind != "voice" && kind != "attachment_offer" && text.isEmpty) {
      return null;
    }
    return StoredChatLine(
      localId: localId,
      text: text,
      outgoing: raw["outgoing"] == true,
      from: raw["from"]?.toString(),
      messageId: raw["message_id"]?.toString(),
      delivery: raw["delivery"]?.toString() ?? "pending",
      createdAtMs: raw["created_at_ms"] is int
          ? raw["created_at_ms"] as int
          : null,
      receivedAtMs: raw["received_at_ms"] is int
          ? raw["received_at_ms"] as int
          : null,
      readAckSent: raw["read_ack_sent"] == true,
      msgKind: kind,
      durationMs: _intField(raw["duration_ms"]),
      audioPath: raw["audio_path"]?.toString(),
      fileName: fileName,
      mimeType: raw["mime_type"]?.toString(),
      sizeBytes: _intField(raw["size_bytes"]),
      localPath: raw["local_path"]?.toString(),
    );
  }

  static String? _fileNameFromAttachmentPreview(String text) {
    final t = text.trim();
    if (!t.startsWith("📎")) return null;
    final name = t.substring("📎".length).trim();
    return name.isEmpty ? null : name;
  }

  static int? _intField(Object? value) {
    if (value is int) return value;
    if (value is num) return value.toInt();
    return null;
  }
}

/// Native transcript snapshot for UI paint (revision + merged lines).
class TranscriptThreadView {
  const TranscriptThreadView({
    required this.revision,
    required this.lines,
    this.hasMore = false,
  });

  final int revision;
  final List<StoredChatLine> lines;

  /// True when older lines exist before this (paginated) window.
  final bool hasMore;
}

/// Local transcript — persisted in **`ghal_bol`**.
///
/// **Daemon platforms (Android/Linux):** `:p2p` / `ghal_bol_core_daemon` owns all
/// transcript **writes** on poll; UI loads read-only via [GhalBolP2p.transcriptLoadThreadView].
class ChatTranscriptStore {
  ChatTranscriptStore._();

  static final Map<String, List<StoredChatLine>> _threadMemoryCache = {};

  static bool get _backgroundOwnsWrites => GhalBolP2p.usesDaemon;

  static String _threadCacheKey(
    String appNamespace,
    Set<String> conversationKeys,
  ) {
    final keys =
        conversationKeys
            .map((e) => e.trim())
            .where((e) => e.isNotEmpty)
            .toList()
          ..sort();
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
    final keys = conversationKeys
        .map((e) => e.trim())
        .where((e) => e.isNotEmpty)
        .toList();
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
    final keys =
        conversationKeys ??
        (conversationKey != null && conversationKey.trim().isNotEmpty
            ? {conversationKey.trim()}
            : <String>{});
    if (keys.isEmpty) return;
    final canon =
        conversationKey?.trim() ?? (keys.length == 1 ? keys.first : "");
    await loadMerged(
      appNamespace: appNamespace,
      conversationKeys: keys,
      cacheUnderConversationKey: canon.isNotEmpty ? canon : null,
    );
  }

  static Future<String> resolvePathForNamespace(String appNamespace) async {
    return GhalBolFfi.transcriptResolvePath(appNamespace) ?? "";
  }

  static Future<TranscriptThreadView> loadThreadView({
    required String appNamespace,
    required Set<String> conversationKeys,
    String? matchInboundFromPeerId,
    String? cacheUnderConversationKey,
    int? limit,
  }) async {
    final r = await GhalBolP2p.transcriptLoadThreadView(
      appNamespace: appNamespace,
      conversationKeys: conversationKeys.toList(),
      matchInboundFromPeerId: matchInboundFromPeerId,
      limit: limit,
    );
    final lines = r.lines
        .map(StoredChatLine.fromJson)
        .whereType<StoredChatLine>()
        .toList();
    final cached = List<StoredChatLine>.from(lines);
    // Only the full (unlimited) view is a complete thread snapshot safe to reuse as
    // a cache for other surfaces (hub warm, peek). A limited window is not.
    if (!r.hasMore) {
      _threadMemoryCache[_threadCacheKey(appNamespace, conversationKeys)] =
          cached;
      final canon = cacheUnderConversationKey?.trim() ?? "";
      if (canon.isNotEmpty) {
        _threadMemoryCache[_threadCacheKey(appNamespace, {canon})] = cached;
      }
    }
    return TranscriptThreadView(
      revision: r.revision,
      lines: lines,
      hasMore: r.hasMore,
    );
  }

  static Future<List<StoredChatLine>> loadMerged({
    required String appNamespace,
    required Set<String> conversationKeys,
    String? matchInboundFromPeerId,

    /// When set, also cache under this single key (for hub warm + peek by canonical conv).
    String? cacheUnderConversationKey,
  }) async {
    final view = await loadThreadView(
      appNamespace: appNamespace,
      conversationKeys: conversationKeys,
      matchInboundFromPeerId: matchInboundFromPeerId,
      cacheUnderConversationKey: cacheUnderConversationKey,
    );
    return view.lines;
  }

  static Future<List<StoredChatLine>> load({
    required String appNamespace,
    required String conversationKey,
  }) => loadMerged(
    appNamespace: appNamespace,
    conversationKeys: {conversationKey},
  );

  static Future<void> appendIfNew({
    required String appNamespace,
    required String conversationKey,
    required StoredChatLine line,
  }) async {
    if (_backgroundOwnsWrites) return;
  }

  static Future<void> patchInboundReadAckSent({
    required String appNamespace,
    required String conversationKey,
    required String messageId,
  }) async {
    if (_backgroundOwnsWrites) return;
  }

  static Future<void> patchOutgoingDelivery({
    required String appNamespace,
    required String conversationKey,
    required String messageId,
    required String delivery,
  }) async {
    if (_backgroundOwnsWrites) return;
  }

  static Future<void> repairCorruptOutgoingDeliveryOnce({
    required String appNamespace,
  }) async {}

  static Future<void> save({
    required String appNamespace,
    required String conversationKey,
    required List<StoredChatLine> lines,
  }) async {
    if (_backgroundOwnsWrites) return;
  }

  static Future<List<StoredChatLine>> trySalvageQuarantinedThread({
    required String appNamespace,
    required String conversationKey,
  }) async =>
      load(appNamespace: appNamespace, conversationKey: conversationKey);
}
