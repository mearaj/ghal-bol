import "dart:async";

import "package:ghal_bol_ui/app_log.dart";
import "package:ghal_bol_ui/chat_transcript_store.dart";
import "package:ghal_bol_ui/ghal_bol_p2p.dart";
import "package:ghal_bol_ui/public_key_hex.dart";
import "package:ghal_bol_ui/saved_contact.dart";

/// Keeps trying to deliver outbound DMs until [ack_received].
class OutboundRetryCoordinator {
  OutboundRetryCoordinator._();

  static final OutboundRetryCoordinator instance = OutboundRetryCoordinator._();

  static const int _tickMs = 2000;
  static const int _minGapMs = 1800;
  static const int _maxAttemptsPerTick = 16;

  final Map<String, _PendingOutbound> _pending = {};
  Timer? _timer;
  Object? _handlerOwner;
  Future<bool> Function(String localId)? _freshSend;

  void attachHandler(Object owner, Future<bool> Function(String localId) sendByLocalId) {
    _handlerOwner = owner;
    _freshSend = sendByLocalId;
  }

  void detachHandler(Object owner) {
    if (_handlerOwner == owner) {
      _handlerOwner = null;
      _freshSend = null;
    }
  }

  void start() {
    _timer ??= Timer.periodic(const Duration(milliseconds: _tickMs), (_) => unawaited(_tick()));
  }

  Future<void> bootstrapFromContacts({
    required String appNamespace,
    required List<SavedContact> contacts,
  }) async {
    for (final c in contacts) {
      final recipientPk = resolvePublicKeyHex(storedHex: c.publicKeyHex);
      if (!isValidPublicKeyHex(recipientPk)) continue;
      final conv = c.conversationKey.trim();
      if (conv.isEmpty) continue;
      final rows = await ChatTranscriptStore.loadMerged(
        appNamespace: appNamespace,
        conversationKeys: c.allConversationKeys,
      );
      for (final row in rows) {
        if (!row.outgoing) continue;
        final d = row.delivery;
        if (d == "delivered" || d == "read") continue;
        final mid = row.messageId?.trim() ?? "";
        if (mid.isEmpty) continue;
        track(
          localId: row.localId,
          messageId: mid,
          recipientPublicKeyHex: recipientPk!,
          text: row.text,
        );
      }
    }
  }

  void stop() {
    _timer?.cancel();
    _timer = null;
    _pending.clear();
    _handlerOwner = null;
    _freshSend = null;
  }

  void track({
    required String localId,
    String? messageId,
    required String recipientPublicKeyHex,
    required String text,
  }) {
    final mid = messageId?.trim() ?? "";
    final key = mid.isNotEmpty ? mid : localId;
    AppLog.instance.flow(
      "Retry",
      "track msg_id=${mid.isNotEmpty ? mid : "(pending)"} local_id=$localId pending=${_pending.length + 1}",
    );
    _pending[key] = _PendingOutbound(
      localId: localId,
      messageId: mid,
      recipientPublicKeyHex: recipientPublicKeyHex,
      text: text,
      lastAttemptMs: 0,
    );
    start();
  }

  void untrack({String? messageId, String? localId}) {
    final mid = messageId?.trim() ?? "";
    if (mid.isNotEmpty) {
      _pending.remove(mid);
    }
    final lid = localId?.trim() ?? "";
    if (lid.isNotEmpty) {
      _pending.remove(lid);
    }
    if (mid.isNotEmpty || lid.isNotEmpty) {
      AppLog.instance.flow(
        "Retry",
        "untrack msg_id=${mid.isNotEmpty ? mid : "-"} local_id=${lid.isNotEmpty ? lid : "-"} pending=${_pending.length}",
      );
    }
  }

  Future<void> _tick() async {
    if (_pending.isEmpty) return;
    if (!await GhalBolP2p.isRunning()) {
      AppLog.instance.flow("Retry", "tick skipped: p2p not running pending=${_pending.length}");
      return;
    }
    final now = DateTime.now().millisecondsSinceEpoch;
    var attempts = 0;
    for (final e in List<_PendingOutbound>.from(_pending.values)) {
      if (attempts >= _maxAttemptsPerTick) break;
      if (now - e.lastAttemptMs < _minGapMs) continue;

      final key = e.messageId.isNotEmpty ? e.messageId : e.localId;
      final entry = _pending[key];
      if (entry == null) continue;

      entry.lastAttemptMs = now;
      attempts++;

      if (e.messageId.isNotEmpty && GhalBolP2p.isRequeueAvailable) {
        AppLog.instance.flow(
          "Retry",
          "requeue msg_id=${e.messageId} pending=${_pending.length}",
        );
        await GhalBolP2p.requeueOutboundDm(
          messageId: e.messageId,
          recipientPublicKeyHex: e.recipientPublicKeyHex,
          text: e.text,
        );
        continue;
      }

      final send = _freshSend;
      if (send != null) {
        await send(e.localId);
      }
    }
  }
}

class _PendingOutbound {
  _PendingOutbound({
    required this.localId,
    required this.messageId,
    required this.recipientPublicKeyHex,
    required this.text,
    required this.lastAttemptMs,
  });

  final String localId;
  String messageId;
  final String recipientPublicKeyHex;
  final String text;
  int lastAttemptMs;
}
