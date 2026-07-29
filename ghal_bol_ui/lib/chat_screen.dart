import "dart:async" show StreamSubscription, Timer, unawaited;
import "dart:io";
import "dart:math" show Random, max, sin, pi;

import "package:audioplayers/audioplayers.dart";
import "package:flutter/foundation.dart" show kIsWeb;
import "package:flutter/material.dart";
import "package:flutter/services.dart";
import "package:file_picker/file_picker.dart";
import "package:ghal_bol_ui/app_log.dart";
import "package:ghal_bol_ui/call/call_controller.dart";
import "package:ghal_bol_ui/ghal_bol_p2p.dart";
import "package:ghal_bol_ui/ghal_bol_ui_session.dart";
import "package:ghal_bol_ui/ghal_bol_connect_invite.dart";
import "package:ghal_bol_ui/identity_alias_store.dart";
import "package:ghal_bol_ui/identity_display_name.dart";
import "package:ghal_bol_ui/invite_scan_screen.dart";
import "package:path_provider/path_provider.dart";
import "package:permission_handler/permission_handler.dart";
import "package:record/record.dart";

import "chat_transcript_store.dart";
import "chat_wallpaper.dart";
import "contact_store.dart";
import "p2p_event_bridge.dart";
import "p2p_link_error_ui.dart";
import "invite_uri_builder.dart";
import "saved_contact.dart";
import "share_invite_screen.dart";
import "package:share_plus/share_plus.dart";
import "package:url_launcher/url_launcher.dart";
import "ghal_bol_constants.dart";
import "dm_ack_validation.dart";
import "dm_delivery_sync.dart";
import "public_key_hex.dart";

class _ComposerSendIntent extends Intent {
  const _ComposerSendIntent();
}

/// One chat surface: you are always reachable locally; optionally you **join someone**
/// using their link/QR, or you **share** yours so they can reach you. No “host/client” choice.
class ChatScreen extends StatefulWidget {
  const ChatScreen({
    super.key,
    required this.libp2pPeerId,
    this.publicKeyHex,

    /// Narrow / stacked layout: back affordance (does not stop the listener).
    this.onLeaveRoom,

    /// When set (e.g. stacked hub), Lock is available without returning to the list pane.
    this.onLock,
    this.localPeerAlias,
    this.aliasNonce = 0,
    this.appNamespace,

    /// When true, Join / Share invitation live on [ChatHubScreen]; this surface is messages only.
    this.networkActionsInHub = false,

    /// Active 1:1 contact (open chat room). Roster metadata only — not the thread id.
    this.activeContact,

    /// Hub: authoritative DM thread (`public_key_hex`). Stable when roster row flickers.
    this.hubThreadKey,

    /// Hub owns P2P poll + [P2pNetworkCoordinator]; this screen only renders one contact.
    this.hubPollsEvents = false,

    /// Called after a successful join so the shell can persist the contact.
    this.onContactJoined,
    this.onHubChatAttach,
    this.onHubChatDetach,
    this.hubPeerStreamReady,
  });

  final String libp2pPeerId;
  final String? publicKeyHex;

  /// Optional owner-chosen label for this device; invitations include it only when non-empty.
  final String? localPeerAlias;
  final VoidCallback? onLeaveRoom;
  final VoidCallback? onLock;

  /// Parent bumps this after alias save so this screen re-reads storage when [localPeerAlias] stays null.
  final int aliasNonce;

  /// Logical app namespace for **`ghal_bol`** prefs (defaults to [kGhalBolAppNamespace]).
  final String? appNamespace;

  /// When true, Join / Share invitation live on [ChatHubScreen]; this surface is messages only.
  final bool networkActionsInHub;

  final SavedContact? activeContact;
  final String? hubThreadKey;
  final bool hubPollsEvents;
  final void Function(SavedContact contact)? onContactJoined;
  final void Function(ChatScreenState state)? onHubChatAttach;
  final void Function(ChatScreenState state)? onHubChatDetach;
  final bool Function(String peerId)? hubPeerStreamReady;

  @override
  State<ChatScreen> createState() => ChatScreenState();
}

enum _MsgDelivery { pending, sent, delivered, read, downloaded, failed }

class _ChatLine {
  _ChatLine({
    required this.localId,
    required this.text,
    this.from,
    this.system = false,
    this.outgoing = false,
    this.messageId,
    this.delivery = _MsgDelivery.pending,
    this.readAckSent = false,
    this.msgKind = "text",
    this.durationMs,
    this.audioPath,
    this.fileName,
    this.mimeType,
    this.sizeBytes,
    this.localPath,
    int? createdAtMs,
  }) : createdAtMs = createdAtMs ?? DateTime.now().millisecondsSinceEpoch;

  final String localId;
  final String text;
  final String? from;
  final bool system;

  /// True for lines we know are from this device (send path or matching [from]).
  final bool outgoing;
  String? messageId;
  _MsgDelivery delivery;

  /// Inbound only: sender confirmed our `ack_read` (local view; see [dm_delivery_sync.dart]).
  bool readAckSent;
  final String msgKind;
  final int? durationMs;
  final String? audioPath;
  final String? fileName;
  final String? mimeType;
  final int? sizeBytes;
  final String? localPath;
  final int createdAtMs;

  bool get _persisted => !system;

  bool get isVoice => msgKind == "voice";
  bool get isAttachment =>
      msgKind == "attachment_offer" ||
      (fileName?.trim().isNotEmpty == true) ||
      text.trimLeft().startsWith("📎");

  StoredChatLine toStored() => StoredChatLine(
    localId: localId,
    text: text,
    outgoing: outgoing,
    from: from,
    messageId: messageId,
    delivery: delivery.name,
    readAckSent: readAckSent,
    createdAtMs: createdAtMs,
    msgKind: msgKind,
    durationMs: durationMs,
    audioPath: audioPath,
    fileName: fileName,
    mimeType: mimeType,
    sizeBytes: sizeBytes,
    localPath: localPath,
  );
}

/// Modern rounded voice waveform (bubble / preview / live recording).
class _VoiceWaveform extends StatelessWidget {
  const _VoiceWaveform({
    required this.levels,
    required this.progress,
    required this.activeColor,
    required this.inactiveColor,
    this.height = 32,
    this.onSeek,
  });

  /// Bar heights in 0..1.
  final List<double> levels;

  /// Played fraction 0..1 (bars before this use [activeColor]).
  final double progress;

  final Color activeColor;
  final Color inactiveColor;
  final double height;
  final ValueChanged<double>? onSeek;

  /// Organic-looking static levels seeded by [seed] (message id / path hash).
  static List<double> synthLevels(int seed, {int count = 36}) {
    final r = Random(seed);
    final out = <double>[];
    var prev = 0.35;
    for (var i = 0; i < count; i++) {
      final t = i / max(1, count - 1);
      // Soft envelope: quieter at the ends, fuller in the middle.
      final envelope = 0.28 + 0.72 * sin(pi * t).clamp(0.0, 1.0);
      final noise = r.nextDouble();
      final next = (prev * 0.55 + noise * 0.45);
      prev = next;
      out.add((0.12 + next * envelope).clamp(0.08, 1.0));
    }
    return out;
  }

  static int seedFrom(String? a, String? b, int? durationMs) {
    final s = "${a ?? ""}|${b ?? ""}|${durationMs ?? 0}";
    return s.hashCode;
  }

  @override
  Widget build(BuildContext context) {
    return LayoutBuilder(
      builder: (context, constraints) {
        final w = constraints.maxWidth;
        return GestureDetector(
          behavior: HitTestBehavior.opaque,
          onTapDown: onSeek == null
              ? null
              : (d) {
                  if (w <= 0) return;
                  onSeek!((d.localPosition.dx / w).clamp(0.0, 1.0));
                },
          onHorizontalDragUpdate: onSeek == null
              ? null
              : (d) {
                  if (w <= 0) return;
                  onSeek!((d.localPosition.dx / w).clamp(0.0, 1.0));
                },
          child: SizedBox(
            height: height,
            width: w,
            child: CustomPaint(
              painter: _VoiceWaveformPainter(
                levels: levels,
                progress: progress.clamp(0.0, 1.0),
                activeColor: activeColor,
                inactiveColor: inactiveColor,
              ),
            ),
          ),
        );
      },
    );
  }
}

class _VoiceWaveformPainter extends CustomPainter {
  _VoiceWaveformPainter({
    required this.levels,
    required this.progress,
    required this.activeColor,
    required this.inactiveColor,
  });

  final List<double> levels;
  final double progress;
  final Color activeColor;
  final Color inactiveColor;

  @override
  void paint(Canvas canvas, Size size) {
    if (levels.isEmpty || size.width <= 0 || size.height <= 0) return;
    const gap = 2.2;
    final n = levels.length;
    final barW = max(2.0, (size.width - gap * (n - 1)) / n);
    final midY = size.height / 2;
    final playedX = size.width * progress;

    for (var i = 0; i < n; i++) {
      final x = i * (barW + gap);
      final h = max(3.0, levels[i] * size.height);
      final rect = RRect.fromRectAndRadius(
        Rect.fromCenter(
          center: Offset(x + barW / 2, midY),
          width: barW,
          height: h,
        ),
        Radius.circular(barW / 2),
      );
      // Soft split: bar is active if its center is before the playhead.
      final active = (x + barW / 2) <= playedX;
      final paint = Paint()
        ..color = active ? activeColor : inactiveColor
        ..style = PaintingStyle.fill
        ..isAntiAlias = true;
      canvas.drawRRect(rect, paint);
    }

    // Playhead pill — only when partially played.
    if (progress > 0.01 && progress < 0.99) {
      final head = Paint()
        ..color = activeColor
        ..style = PaintingStyle.fill
        ..isAntiAlias = true;
      canvas.drawRRect(
        RRect.fromRectAndRadius(
          Rect.fromCenter(
            center: Offset(playedX, midY),
            width: 3.2,
            height: size.height * 0.92,
          ),
          const Radius.circular(2),
        ),
        head,
      );
    }
  }

  @override
  bool shouldRepaint(covariant _VoiceWaveformPainter old) {
    return old.progress != progress ||
        old.activeColor != activeColor ||
        old.inactiveColor != inactiveColor ||
        old.levels.length != levels.length ||
        !_listEq(old.levels, levels);
  }

  static bool _listEq(List<double> a, List<double> b) {
    if (identical(a, b)) return true;
    if (a.length != b.length) return false;
    for (var i = 0; i < a.length; i++) {
      if ((a[i] - b[i]).abs() > 0.001) return false;
    }
    return true;
  }
}

class _VoiceBubble extends StatefulWidget {
  const _VoiceBubble({
    required this.line,
    required this.foreground,
    required this.metaColor,
  });

  final _ChatLine line;
  final Color foreground;
  final Color metaColor;

  @override
  State<_VoiceBubble> createState() => _VoiceBubbleState();
}

class _VoiceBubbleState extends State<_VoiceBubble> {
  final AudioPlayer _player = AudioPlayer();
  bool _playing = false;
  bool _loading = false;
  Duration _position = Duration.zero;
  Duration _duration = Duration.zero;
  late final List<double> _levels;

  @override
  void initState() {
    super.initState();
    _levels = _VoiceWaveform.synthLevels(
      _VoiceWaveform.seedFrom(
        widget.line.messageId,
        widget.line.localId,
        widget.line.durationMs,
      ),
      count: 40,
    );
    final ms = widget.line.durationMs ?? 0;
    if (ms > 0) _duration = Duration(milliseconds: ms);
    _player.onPlayerComplete.listen((_) {
      if (mounted) {
        setState(() {
          _playing = false;
          _position = Duration.zero;
        });
      }
    });
    _player.onPositionChanged.listen((pos) {
      if (mounted) setState(() => _position = pos);
    });
    _player.onDurationChanged.listen((d) {
      if (mounted && d.inMilliseconds > 0) setState(() => _duration = d);
    });
  }

  @override
  void dispose() {
    _player.dispose();
    super.dispose();
  }

  Future<String?> _playablePath() async {
    final candidates = <String>[];
    final audioPath = widget.line.audioPath?.trim() ?? "";
    final localPath = widget.line.localPath?.trim() ?? "";
    if (audioPath.endsWith(".opus")) {
      candidates.add("${audioPath.substring(0, audioPath.length - 5)}.wav");
    }
    if (localPath.endsWith(".opus")) {
      candidates.add("${localPath.substring(0, localPath.length - 5)}.wav");
    }
    candidates.addAll([localPath, audioPath]);
    for (final path in candidates.where((p) => p.isNotEmpty)) {
      if (await File(path).exists()) return path;
    }
    return null;
  }

  Future<void> _toggle() async {
    if (_loading) return;
    if (_playing) {
      await _player.pause();
      if (mounted) setState(() => _playing = false);
      return;
    }
    setState(() => _loading = true);
    final path = await _playablePath();
    if (!mounted) return;
    setState(() => _loading = false);
    if (path == null) return;
    if (_position == Duration.zero) {
      await _player.play(DeviceFileSource(path));
    } else {
      await _player.resume();
    }
    if (mounted) setState(() => _playing = true);
  }

  Future<void> _seekFraction(double fraction) async {
    final totalMs = _duration.inMilliseconds > 0
        ? _duration.inMilliseconds
        : (widget.line.durationMs ?? 0);
    if (totalMs <= 0) return;
    final target = Duration(milliseconds: (totalMs * fraction).round());
    try {
      await _player.seek(target);
    } catch (_) {
      return;
    }
    if (mounted) setState(() => _position = target);
  }

  @override
  Widget build(BuildContext context) {
    final totalMs = _duration.inMilliseconds > 0
        ? _duration.inMilliseconds
        : (widget.line.durationMs ?? 0);
    final progress = totalMs > 0
        ? (_position.inMilliseconds / totalMs).clamp(0.0, 1.0)
        : 0.0;
    final label = _playing || _position.inMilliseconds > 0
        ? ChatScreenState._formatDurationMs(_position.inMilliseconds)
        : ChatScreenState._formatDurationMs(widget.line.durationMs);
    return ConstrainedBox(
      constraints: const BoxConstraints(minWidth: 200),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          IconButton.filled(
            onPressed: _toggle,
            icon: _loading
                ? const SizedBox(
                    width: 18,
                    height: 18,
                    child: CircularProgressIndicator(strokeWidth: 2),
                  )
                : Icon(_playing ? Icons.pause_rounded : Icons.play_arrow_rounded),
            color: widget.foreground,
            style: IconButton.styleFrom(
              backgroundColor: widget.metaColor.withValues(alpha: 0.18),
              minimumSize: const Size(42, 42),
            ),
          ),
          const SizedBox(width: 10),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              mainAxisSize: MainAxisSize.min,
              children: [
                _VoiceWaveform(
                  levels: _levels,
                  progress: progress,
                  activeColor: widget.foreground,
                  inactiveColor: widget.foreground.withValues(alpha: 0.28),
                  height: 28,
                  onSeek: _seekFraction,
                ),
                const SizedBox(height: 4),
                Text(
                  label,
                  style: TextStyle(
                    color: widget.metaColor,
                    fontSize: 11,
                    fontWeight: FontWeight.w600,
                    fontFeatures: const [FontFeature.tabularFigures()],
                  ),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

class ChatScreenState extends State<ChatScreen> with WidgetsBindingObserver {
  final _msgCtrl = TextEditingController();
  final ScrollController _scrollController = ScrollController();
  final List<_ChatLine> _lines = [];

  /// Latest trust flags for [widget.activeContact] (refreshed from [ContactStore]).
  SavedContact? _roomContact;

  /// After you join using someone else’s invite, sealed messages use their encryption pubkey.
  GhalBolConnectInvite? _remoteInvite;

  /// Host may learn guest public key from native `peer_identified` (derived from libp2p PeerId or invite).
  String? _learnedRemotePublicKeyHex;

  final Set<String> _seenLibp2pConnections = {};
  final Set<String> _ackedReadIds = {};

  /// Acks can arrive before [messageId] is assigned on the outgoing bubble.
  final Set<String> _pendingDeliveredAckRefs = {};
  String? _transcriptLoadedKey;
  bool _loadingTranscript = false;

  /// Room switches while a load is in flight must not be dropped, or the new room
  /// keeps the spinner with nothing behind it.
  bool _transcriptReloadQueued = false;
  bool _transcriptReloadQueuedForce = false;

  /// Offer ids whose bytes native is still pulling over the peer connection.
  final Set<String> _downloadingOfferIds = {};
  final List<Map<String, dynamic>> _bufferedHubDmEvents = [];

  /// Live UI guard — never paint the same wire [message_id] twice this session.
  final Set<String> _uiSeenMessageIds = {};
  int _reloadGeneration = 0;
  int _paintedTranscriptRevision = 0;

  /// Newest-N transcript window; grows by [_transcriptPageSize] on scroll-up so a
  /// long history is not loaded or rendered all at once.
  static const int _transcriptPageSize = 50;
  int _transcriptWindowLimit = _transcriptPageSize;
  bool _transcriptHasMore = false;
  bool _loadingOlderPage = false;
  Future<void> _transcriptFlushChain = Future<void>.value();
  Future<void> _transcriptSyncChain = Future<void>.value();

  String? _chatError;

  bool _peerMatchesActive(String? wirePeerId) {
    final wire = wirePeerId?.trim() ?? "";
    if (wire.isEmpty) return false;
    final pk = _recipientPublicKeyHex();
    if (isValidPublicKeyHex(pk) &&
        (publicKeysEqual(pk, wire) ||
            libp2pWireMatchesContactPublicKey(
              wirePeerId: wire,
              contactPublicKeyHex: pk!,
            ))) {
      return true;
    }
    final ac = widget.activeContact;
    if (ac != null && ac.hasPublicKey) {
      if (publicKeysEqual(ac.publicKeyHex, wire)) return true;
      return libp2pWireMatchesContactPublicKey(
        wirePeerId: wire,
        contactPublicKeyHex: ac.publicKeyHex,
      );
    }
    return false;
  }

  Timer? _saveTranscriptDebounce;
  Timer? _fullTranscriptSaveTimer;
  Timer? _transcriptSyncDebounce;
  final Map<String, _MsgDelivery> _pendingDeliveryPatches = {};
  final Set<String> _transcriptFlushedLocalIds = {};
  static final _localIdRandom = Random();
  static const int _maxVoiceDurationMs = 120000;
  static const int _minVoiceDurationMs = 300;
  final AudioRecorder _voiceRecorder = AudioRecorder();
  bool _isRecordingVoice = false;
  DateTime? _recordingStartedAt;
  Duration _recordingElapsed = Duration.zero;
  Timer? _recordingTimer;
  Timer? _recordingAutoStopTimer;
  String? _recordingPath;

  // Captured clip awaiting explicit send/discard (modern preview flow).
  String? _voicePreviewPath;
  int _voicePreviewDurationMs = 0;
  final AudioPlayer _voicePreviewPlayer = AudioPlayer();
  bool _voicePreviewPlaying = false;
  Duration _voicePreviewPosition = Duration.zero;
  bool _voiceSending = false;
  List<double> _voicePreviewLevels = const [];
  /// Live mic amplitude bars while recording (rolling window).
  final List<double> _liveVoiceLevels = List<double>.filled(40, 0.12);
  StreamSubscription<Amplitude>? _voiceAmplitudeSub;

  /// When [ChatScreen.localPeerAlias] is null (e.g. legacy route), lift from device store.
  String? _storeAliasLift;

  bool get _joinedRemote => _remoteInvite != null;

  String? get _callPeerPkHex {
    final pk = widget.activeContact?.publicKeyHex ?? widget.publicKeyHex;
    final wire = resolvePublicKeyHex(storedHex: pk);
    return wire != null && isValidPublicKeyHex(wire) ? wire : null;
  }

  String _callPeerDisplayName() {
    final ac = widget.activeContact;
    if (ac != null) {
      return ghalBolIdName(
        publicKeyHex: ac.publicKeyHex,
        customAlias: ac.displayAlias,
      );
    }
    return ghalBolIdName(publicKeyHex: widget.publicKeyHex, customAlias: null);
  }

  String? get _effectiveCustomAlias =>
      ghalSanitizePeerAlias(widget.localPeerAlias) ??
      ghalSanitizePeerAlias(_storeAliasLift);

  static bool _hexEq(String a, String b) =>
      a.trim().toLowerCase() == b.trim().toLowerCase();

  bool _hasInboundMessageId(String refId) =>
      _lines.any((l) => !l.outgoing && l.messageId?.trim() == refId);

  /// Sender confirmed our `ack_read` for their inbound text (ref_id = their message id on wire).
  void _markInboundReadAckConfirmed(String refId) {
    final mid = refId.trim();
    if (mid.isEmpty || _ackedReadIds.contains(mid)) return;
    _ackedReadIds.add(mid);
    var changed = false;
    for (final l in _lines) {
      if (l.outgoing || l.messageId?.trim() != mid) continue;
      if (!l.readAckSent) {
        l.readAckSent = true;
        changed = true;
      }
    }
    if (!widget.hubPollsEvents) {
      unawaited(
        ChatTranscriptStore.patchInboundReadAckSent(
          appNamespace: _resolvedAppNamespace,
          conversationKey: _conversationKey(),
          messageId: mid,
        ),
      );
    }
    if (changed && mounted) setState(() {});
  }

  bool _inboundAckFromActivePeer(Map<String, dynamic> ev) {
    final from = ev["from"]?.toString().trim() ?? "";
    if (from.isEmpty) return false;
    final senderPk = contactPublicKeyHexFromEvent(ev);
    final ac = widget.activeContact;
    final matchesPeer = _peerMatchesActive(from);
    final matchesKeys = dmAckSenderMatchesPeerKeys(
      senderPublicKeyHex: senderPk,
      contact: ac,
      learnedRemotePublicKeyHex: _recipientPublicKeyHex(),
      invitePublicKeyHex: _remoteInvite?.publicKeyHex,
      ackFromPeerId: from,
    );
    if (!matchesPeer && !matchesKeys) return false;
    if (isValidPublicKeyHex(senderPk)) {
      _applyRemotePeerKeys(senderPk, peerId: from);
    }
    return true;
  }

  bool _pollEventMatchesOpenThread(Map<String, dynamic> ev) {
    if (!widget.hubPollsEvents) return true;
    final evKey = ev["conversation_key"]?.toString().trim().toLowerCase() ?? "";
    if (evKey.isEmpty) return true;
    final pk = _recipientPublicKeyHex()?.trim().toLowerCase() ?? "";
    if (isValidPublicKeyHex(pk) && evKey == pk) return true;
    final conv = _conversationKey().trim().toLowerCase();
    return conv.isNotEmpty && evKey == conv;
  }

  /// History sync added a hub-side filter here that dropped `peer_identified` / `chat_ready`
  /// and many `dm_message` events — live chat must see every event from the hub poll.
  void ingestP2pEvent(Map<String, dynamic> ev) {
    if (_loadingTranscript && ev["kind"]?.toString() == "dm_message") {
      final mk = ev["msg_kind"]?.toString() ?? "";
      if (mk == "text" || mk == "voice") {
        _bufferedHubDmEvents.add(ev);
        return;
      }
    }
    if (widget.hubPollsEvents && ev["kind"]?.toString() == "dm_message") {
      final mk = ev["msg_kind"]?.toString() ?? "";
      if (mk == "text" ||
          mk == "voice" ||
          mk == "attachment_offer" ||
          isRecipientOutboundAckKind(mk) ||
          mk == kSenderConfirmedReadReceipt) {
        if (_pollEventMatchesOpenThread(ev)) {
          _scheduleTranscriptSync(
            force: mk == "text" ||
                mk == "voice" ||
                mk == "attachment_offer" ||
                isRecipientOutboundAckKind(mk),
          );
        }
        return;
      }
    }
    if (ev["kind"]?.toString() == "dm_message" &&
        (ev["msg_kind"]?.toString() == "text" ||
            ev["msg_kind"]?.toString() == "voice" ||
            ev["msg_kind"]?.toString() == "attachment_offer")) {
      final mid = ev["id"]?.toString().trim() ?? "";
      if (mid.isNotEmpty &&
          _uiSeenMessageIds.contains(mid) &&
          _hasMessageId(mid)) {
        return;
      }
    }
    _handleEvent(ev);
    if (ev["stores_updated"] == true &&
        ev["kind"]?.toString() == "dm_message" &&
        isRecipientOutboundAckKind(ev["msg_kind"]?.toString() ?? "")) {
      _scheduleTranscriptSync(force: true);
    }
  }

  /// Hub attached this surface — register peer + merge transcript (optional full reload).
  void onHubReattached({bool reloadTranscript = false}) {
    unawaited(_onHubReattached(reloadTranscript: reloadTranscript));
  }

  void _restoreHubLinkState() {
    final pk = _recipientPublicKeyHex();
    if (!isValidPublicKeyHex(pk)) return;
    // Stream ready only when DM stream is open — not mere TCP connect.
    final hubReady =
        widget.hubPeerStreamReady?.call(pk!) ??
        P2pEventBridge.instance.isStreamReady(pk!);
    if (hubReady && mounted) {
      setState(() {
        _seenLibp2pConnections.add(pk!);
        _chatError = null;
      });
    }
  }

  Future<void> _onHubReattached({bool reloadTranscript = false}) async {
    await _registerActiveDmPeer();
    _restoreHubLinkState();
    final key = _conversationKey();
    final hasVisibleLines = _lines.any((l) => !l.system);
    final needsFullLoad =
        reloadTranscript || _transcriptLoadedKey != key || !hasVisibleLines;
    if (needsFullLoad) {
      _paintCachedTranscriptIfAny();
      await _reloadTranscriptForConversation(force: reloadTranscript);
    }
    await _syncOpenChatToNativeAsync();
    if (!mounted || _loadingTranscript) return;
    await syncTranscriptView(force: needsFullLoad);
  }

  static final Map<String, String> _registeredDmFingerprints = {};

  Future<void> _registerActiveDmPeer() async {
    if (!await GhalBolP2p.isRunning()) return;
    final pk = resolvePublicKeyHex(storedHex: _recipientPublicKeyHex());
    if (!isValidPublicKeyHex(pk)) return;
    final pkNorm = pk!.toLowerCase();
    if (_registeredDmFingerprints[pkNorm] == pkNorm) return;
    _registeredDmFingerprints[pkNorm] = pkNorm;
    await GhalBolP2p.registerDmPeer(pkNorm);
  }

  bool _peerIdentifiedMatchesContact(String publicKeyHex, SavedContact? ac) {
    if (ac == null) return true;
    if (!ac.hasPublicKey) return true;
    return _hexEq(publicKeyHex, ac.publicKeyHex);
  }

  Future<void> _syncForegroundPeerToNative() async {
    // Hub chat uses IndexedStack: room can be hidden while this widget stays mounted.
    // ChatHubScreen owns foreground when hubPollsEvents (IndexedStack keeps us mounted off-room).
    if (widget.hubPollsEvents) return;
    if (!await GhalBolP2p.isRunning()) return;
    final pk = _recipientPublicKeyHex();
    GhalBolUiSession.setRoom(isValidPublicKeyHex(pk) ? pk : null);
  }

  void _drainP2pAfterFrame() {
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) P2pEventBridge.instance.drainNow();
    });
  }

  static int _eventCreatedAtMs(Map<String, dynamic> ev) {
    final v = ev["created_at_ms"];
    if (v is int && v > 0) return v;
    if (v is num && v > 0) return v.toInt();
    return DateTime.now().millisecondsSinceEpoch;
  }

  static int? _eventInt(Map<String, dynamic> ev, String key) {
    final v = ev[key];
    if (v is int) return v;
    if (v is num) return v.toInt();
    return null;
  }

  void _sortLinesByTime() {
    _lines.sort((a, b) {
      final c = a.createdAtMs.compareTo(b.createdAtMs);
      if (c != 0) return c;
      final am = a.messageId?.trim() ?? "";
      final bm = b.messageId?.trim() ?? "";
      if (am.isNotEmpty && bm.isNotEmpty) {
        final mid = am.compareTo(bm);
        if (mid != 0) return mid;
      }
      return a.localId.compareTo(b.localId);
    });
  }

  void _flushBufferedHubDmEvents() {
    if (_bufferedHubDmEvents.isEmpty) return;
    final batch = List<Map<String, dynamic>>.from(_bufferedHubDmEvents);
    _bufferedHubDmEvents.clear();
    for (final ev in batch) {
      ingestP2pEvent(ev);
    }
    _dedupeLinesByMessageId();
    _sortLinesByTime();
  }

  static String _lineContentFingerprint(_ChatLine l) {
    final content = l.isVoice
        ? "voice|${l.durationMs ?? 0}|${l.audioPath ?? l.localPath ?? ""}"
        : l.isAttachment
            ? "attachment|${l.fileName ?? ""}|${l.sizeBytes ?? 0}|${l.localPath ?? ""}"
            : "text|${l.text.trim()}";
    if (l.outgoing) return "1|$content";
    final from = l.from?.trim() ?? "";
    return "0|$from|$content";
  }

  static String? _outboundBubbleKey(_ChatLine l) {
    if (!l.outgoing || l.system) return null;
    if (l.createdAtMs <= 0) return null;
    final content = l.isVoice
        ? "voice|${l.durationMs ?? 0}|${l.localPath ?? l.audioPath ?? ""}"
        : l.isAttachment
            ? "attachment|${l.fileName ?? ""}|${l.sizeBytes ?? 0}|${l.localPath ?? ""}"
            : "text|${l.text.trim()}";
    return "$content|${l.createdAtMs}";
  }

  _ChatLine _pickBetterChatLine(_ChatLine a, _ChatLine b) {
    final amid = a.messageId?.trim() ?? "";
    final bmid = b.messageId?.trim() ?? "";
    if (amid.isEmpty && bmid.isNotEmpty) return b;
    if (bmid.isEmpty && amid.isNotEmpty) return a;
    if (a.outgoing &&
        b.outgoing &&
        _deliveryRank(b.delivery) > _deliveryRank(a.delivery)) {
      return b;
    }
    if (b.createdAtMs < a.createdAtMs) return b;
    return a;
  }

  /// One bubble per message ([messageId], or same text/from/direction within 10s).
  void _dedupeLinesByMessageId() {
    final systems = <_ChatLine>[];
    final byMid = <String, _ChatLine>{};
    final noMid = <_ChatLine>[];
    for (final l in _lines) {
      if (l.system) {
        systems.add(l);
        continue;
      }
      final mid = l.messageId?.trim() ?? "";
      if (mid.isNotEmpty) {
        final prev = byMid[mid];
        byMid[mid] = prev == null ? l : _pickBetterChatLine(prev, l);
      } else {
        noMid.add(l);
      }
    }
    final keptNoMid = <_ChatLine>[];
    for (final l in noMid) {
      final fp = _lineContentFingerprint(l);
      var dup = false;
      for (final m in byMid.values) {
        if (_lineContentFingerprint(m) != fp) continue;
        if ((l.createdAtMs - m.createdAtMs).abs() > 10000) continue;
        dup = true;
        break;
      }
      if (!dup) {
        for (final o in keptNoMid) {
          if (_lineContentFingerprint(o) != fp) continue;
          if ((l.createdAtMs - o.createdAtMs).abs() > 10000) continue;
          dup = true;
          if (_pickBetterChatLine(o, l) == l) {
            keptNoMid[keptNoMid.indexOf(o)] = l;
          }
          break;
        }
      }
      if (!dup) keptNoMid.add(l);
    }
    final byOutboundBubble = <String, _ChatLine>{};
    final rest = <_ChatLine>[];
    for (final l in [...byMid.values, ...keptNoMid]) {
      final ob = _outboundBubbleKey(l);
      if (ob == null) {
        rest.add(l);
        continue;
      }
      final prev = byOutboundBubble[ob];
      byOutboundBubble[ob] = prev == null ? l : _pickBetterChatLine(prev, l);
    }
    for (final l in byOutboundBubble.values) {
      final mid = l.messageId?.trim() ?? "";
      if (mid.isNotEmpty) _uiSeenMessageIds.add(mid);
    }
    for (final l in rest) {
      final mid = l.messageId?.trim() ?? "";
      if (mid.isNotEmpty) _uiSeenMessageIds.add(mid);
    }
    final merged = <_ChatLine>[...systems, ...rest, ...byOutboundBubble.values];
    if (merged.length == _lines.length) return;
    _lines
      ..clear()
      ..addAll(merged);
  }

  bool _tryAddTextLine({
    required String from,
    required String text,
    required bool outgoing,
    String? messageId,
    required int createdAtMs,
  }) {
    final mid = messageId?.trim() ?? "";
    if (mid.isNotEmpty) {
      if (_uiSeenMessageIds.contains(mid) || _hasMessageId(mid)) return false;
    }
    final probe = _ChatLine(
      localId: "",
      text: text,
      from: from,
      outgoing: outgoing,
      createdAtMs: createdAtMs,
    );
    final ob = _outboundBubbleKey(probe);
    for (final l in _lines) {
      if (l.system || l.outgoing != outgoing) continue;
      if (ob != null && outgoing && _outboundBubbleKey(l) == ob) return false;
      if (_lineContentFingerprint(l) != _lineContentFingerprint(probe)) {
        continue;
      }
      if ((l.createdAtMs - createdAtMs).abs() > 10000) continue;
      return false;
    }
    _lines.add(
      _ChatLine(
        localId: _newLocalId(),
        from: from,
        text: text,
        outgoing: outgoing,
        messageId: mid.isEmpty ? null : mid,
        createdAtMs: createdAtMs,
      ),
    );
    if (mid.isNotEmpty) _uiSeenMessageIds.add(mid);
    return true;
  }

  bool _tryAddVoiceLine({
    required String from,
    required bool outgoing,
    String? messageId,
    required int createdAtMs,
    int? durationMs,
    String? audioPath,
    String? localPath,
    String? fileName,
    String? mimeType,
    int? sizeBytes,
  }) {
    final mid = messageId?.trim() ?? "";
    if (mid.isNotEmpty) {
      if (_uiSeenMessageIds.contains(mid) || _hasMessageId(mid)) return false;
    }
    final probe = _ChatLine(
      localId: "",
      text: "",
      from: from,
      outgoing: outgoing,
      msgKind: "voice",
      durationMs: durationMs,
      audioPath: audioPath,
      localPath: localPath,
      fileName: fileName,
      mimeType: mimeType,
      sizeBytes: sizeBytes,
      createdAtMs: createdAtMs,
    );
    final ob = _outboundBubbleKey(probe);
    for (final l in _lines) {
      if (l.system || l.outgoing != outgoing) continue;
      if (ob != null && outgoing && _outboundBubbleKey(l) == ob) return false;
      if (_lineContentFingerprint(l) != _lineContentFingerprint(probe)) {
        continue;
      }
      if ((l.createdAtMs - createdAtMs).abs() > 10000) continue;
      return false;
    }
    _lines.add(
      _ChatLine(
        localId: _newLocalId(),
        from: from,
        text: "",
        outgoing: outgoing,
        messageId: mid.isEmpty ? null : mid,
        msgKind: "voice",
        durationMs: durationMs,
        audioPath: audioPath,
        localPath: localPath,
        fileName: fileName,
        mimeType: mimeType,
        sizeBytes: sizeBytes,
        createdAtMs: createdAtMs,
      ),
    );
    if (mid.isNotEmpty) _uiSeenMessageIds.add(mid);
    return true;
  }

  bool _tryAddAttachmentLine({
    required String from,
    required bool outgoing,
    String? messageId,
    required int createdAtMs,
    String? text,
    String? fileName,
    String? mimeType,
    int? sizeBytes,
    String? localPath,
  }) {
    final mid = messageId?.trim() ?? "";
    if (mid.isNotEmpty) {
      if (_uiSeenMessageIds.contains(mid) || _hasMessageId(mid)) return false;
    }
    final probe = _ChatLine(
      localId: "",
      text: text ?? "",
      from: from,
      outgoing: outgoing,
      msgKind: "attachment_offer",
      fileName: fileName,
      mimeType: mimeType,
      sizeBytes: sizeBytes,
      localPath: localPath,
      createdAtMs: createdAtMs,
    );
    final ob = _outboundBubbleKey(probe);
    for (final l in _lines) {
      if (l.system || l.outgoing != outgoing) continue;
      if (ob != null && outgoing && _outboundBubbleKey(l) == ob) return false;
      if (_lineContentFingerprint(l) != _lineContentFingerprint(probe)) continue;
      if ((l.createdAtMs - createdAtMs).abs() > 10000) continue;
      return false;
    }
    _lines.add(
      _ChatLine(
        localId: _newLocalId(),
        from: from,
        text: text ?? "",
        outgoing: outgoing,
        messageId: mid.isEmpty ? null : mid,
        msgKind: "attachment_offer",
        fileName: fileName,
        mimeType: mimeType,
        sizeBytes: sizeBytes,
        localPath: localPath,
        createdAtMs: createdAtMs,
      ),
    );
    if (mid.isNotEmpty) _uiSeenMessageIds.add(mid);
    return true;
  }

  /// Restore **local** views from disk — not mirrored from the other peer.
  void _seedAckStateFromTranscript(List<StoredChatLine> rows) {
    for (final r in rows) {
      final mid = r.messageId?.trim();
      if (mid == null || mid.isEmpty) continue;
      if (r.outgoing) {
        // Cached recipient decisions on our outbound lines only.
        if (r.delivery == "read") {
          _updateOutgoingDelivery(mid, _MsgDelivery.read);
        } else if (r.delivery == "delivered") {
          _updateOutgoingDelivery(mid, _MsgDelivery.delivered);
        } else if (r.delivery == "sent") {
          _updateOutgoingDelivery(mid, _MsgDelivery.sent);
        }
        continue;
      }
      if (r.readAckSent) {
        _ackedReadIds.add(mid);
        for (final l in _lines) {
          if (!l.outgoing && l.messageId?.trim() == mid) {
            l.readAckSent = true;
          }
        }
      }
    }
  }

  /// Tell native which peer is foreground; outbox resend + read receipts run in the P2P service.
  void _syncOpenChatToNative() {
    unawaited(_syncOpenChatToNativeAsync());
  }

  Future<void> _syncOpenChatToNativeAsync() async {
    if (!mounted) return;
    final running = await GhalBolP2p.isRunning();
    if (!running) {
      AppLog.instance.flow("Chat", "syncOpenChat skipped: p2p not running");
      return;
    }
    await _registerActiveDmPeer();
    if (!widget.hubPollsEvents) {
      await _syncForegroundPeerToNative();
    }
    AppLog.instance.flow(
      "Chat",
      "syncOpenChat ok hubPolls=${widget.hubPollsEvents} pk=${_recipientPublicKeyHex() ?? "(none)"} conv=${_conversationKey()}",
    );
    _drainP2pAfterFrame();
    if (widget.hubPollsEvents) {
      final key = _conversationKey();
      if (key == "solo") return;
      ChatTranscriptStore.invalidateThreadCache(
        appNamespace: _resolvedAppNamespace,
        conversationKeys: _conversationKeysForLoad(),
      );
      unawaited(syncTranscriptView());
    }
  }

  /// Hub lost libp2p stream / dial for the open contact — allow ack catch-up on reconnect.
  void onHubPeerLinkLost() {
    AppLog.instance.w(
      "Chat",
      "hub peer link lost pk=${_recipientPublicKeyHex() ?? "(none)"}",
    );
    _onActivePeerLinkLost();
    if (mounted) setState(() {});
  }

  void _onActivePeerLinkLost() {}

  String? _hubThreadPublicKeyHex() {
    final hub = widget.hubThreadKey?.trim().toLowerCase() ?? "";
    if (isValidPublicKeyHex(hub)) return hub;
    return null;
  }

  String _threadKeyForWidget(ChatScreen w) {
    final hub = w.hubThreadKey?.trim().toLowerCase() ?? "";
    if (isValidPublicKeyHex(hub)) return hub;
    final rc = _roomContact;
    if (rc != null && rc.conversationKey.isNotEmpty) return rc.conversationKey;
    return w.activeContact?.conversationKey ?? "";
  }

  String? _recipientPublicKeyHex() {
    final hub = _hubThreadPublicKeyHex();
    if (hub != null) return hub;
    final learned = _learnedRemotePublicKeyHex?.trim();
    if (isValidPublicKeyHex(learned)) return learned;
    final rc = _roomContact;
    if (rc != null) {
      final pk = resolvePublicKeyHex(storedHex: rc.publicKeyHex);
      if (isValidPublicKeyHex(pk)) return pk;
    }
    final ac = widget.activeContact;
    if (ac != null) {
      final pk = resolvePublicKeyHex(storedHex: ac.publicKeyHex);
      if (isValidPublicKeyHex(pk)) return pk;
    }
    if (_joinedRemote && _remoteInvite != null) {
      final pk = resolvePublicKeyHex(storedHex: _remoteInvite!.publicKeyHex);
      if (isValidPublicKeyHex(pk)) return pk;
    }
    return null;
  }

  void _stashRemotePublicKey(String publicKeyHex) {
    if (!isValidPublicKeyHex(publicKeyHex)) return;
    if (!mounted) return;
    setState(() => _learnedRemotePublicKeyHex = publicKeyHex.trim());
  }

  Future<void> _applyRemotePeerKeys(
    String publicKeyHex, {
    String? peerId,
  }) async {
    if (!isValidPublicKeyHex(publicKeyHex)) return;
    final pk = publicKeyHex.trim();
    _stashRemotePublicKey(pk);
    if (await GhalBolP2p.isRunning()) {
      await GhalBolP2p.registerDmPeer(pk);
    }
    if (!mounted) return;
    setState(() => _chatError = null);
  }

  /// Canonical thread key for **writes** — always `public_key_hex`.
  String _conversationKey() {
    final hub = _hubThreadPublicKeyHex();
    if (hub != null) return hub;
    final rc = _roomContact;
    if (rc != null && rc.conversationKey.isNotEmpty) return rc.conversationKey;
    final ac = widget.activeContact;
    if (ac != null && ac.conversationKey.isNotEmpty) return ac.conversationKey;
    final peer = _recipientPublicKeyHex();
    if (isValidPublicKeyHex(peer)) return peer!;
    return "solo";
  }

  /// All thread keys for **reads** (pk + legacy libp2p PeerId bucket on disk).
  Set<String> _conversationKeysForLoad() {
    final keys = <String>{};
    final hub = _hubThreadPublicKeyHex();
    if (hub != null) {
      keys.addAll(SavedContact(publicKeyHex: hub).allConversationKeys);
    }
    final rc = _roomContact;
    if (rc != null) keys.addAll(rc.allConversationKeys);
    final ac = widget.activeContact;
    if (ac != null) keys.addAll(ac.allConversationKeys);
    final pk = _recipientPublicKeyHex();
    if (isValidPublicKeyHex(pk)) keys.add(pk!.trim());
    keys.removeWhere((k) => k.isEmpty || k == "solo");
    if (keys.isEmpty) keys.add("solo");
    return keys;
  }

  String _newLocalId() =>
      "${DateTime.now().microsecondsSinceEpoch.toRadixString(16)}_${_localIdRandom.nextInt(0xFFFFFF).toRadixString(16)}";

  static int _deliveryRank(_MsgDelivery d) {
    switch (d) {
      case _MsgDelivery.pending:
        return 0;
      case _MsgDelivery.failed:
        return 1;
      case _MsgDelivery.sent:
        return 2;
      case _MsgDelivery.delivered:
        return 3;
      case _MsgDelivery.read:
        return 4;
      case _MsgDelivery.downloaded:
        return 4;
    }
  }

  static _MsgDelivery _deliveryFromStored(StoredChatLine s) {
    switch (s.delivery) {
      case "read":
        return _MsgDelivery.read;
      case "downloaded":
        return _MsgDelivery.downloaded;
      case "delivered":
        return _MsgDelivery.delivered;
      case "sent":
        return _MsgDelivery.sent;
      case "failed":
        return _MsgDelivery.failed;
      default:
        return _MsgDelivery.pending;
    }
  }

  void _applyMonotonicDelivery(_ChatLine line, _MsgDelivery state) {
    if (_deliveryRank(state) <= _deliveryRank(line.delivery)) return;
    line.delivery = state;
    final mid = line.messageId?.trim() ?? "";
    if (mid.isNotEmpty) {
      _pendingDeliveryPatches[mid] = state;
    }
  }

  void _applyDeliveryPatchesFromStoredRows(List<StoredChatLine> rows) {
    for (final r in rows) {
      if (!r.outgoing) continue;
      final mid = r.messageId?.trim() ?? "";
      if (mid.isEmpty) continue;
      final d = _deliveryFromStored(r);
      for (final l in _lines) {
        if (!l.outgoing || l.messageId?.trim() != mid) continue;
        _applyMonotonicDelivery(l, d);
        break;
      }
    }
  }

  /// Coalesce burst poll events into one native transcript snapshot read.
  void _scheduleTranscriptSync({bool force = false}) {
    if (force) {
      _transcriptSyncDebounce?.cancel();
      unawaited(syncTranscriptView(force: true));
      return;
    }
    _transcriptSyncDebounce?.cancel();
    _transcriptSyncDebounce = Timer(const Duration(milliseconds: 100), () {
      if (!mounted) return;
      unawaited(syncTranscriptView());
    });
  }

  /// Replace painted lines from native transcript (revision-guarded full snapshot).
  Future<void> syncTranscriptView({bool force = false}) async {
    if (!mounted) return;
    final key = _conversationKey();
    if (widget.hubPollsEvents && key == "solo") return;

    _transcriptSyncChain = _transcriptSyncChain.then((_) async {
      if (!mounted) return;
      ChatTranscriptStore.invalidateThreadCache(
        appNamespace: _resolvedAppNamespace,
        conversationKeys: _conversationKeysForLoad(),
      );
      try {
        final view = await ChatTranscriptStore.loadThreadView(
          appNamespace: _resolvedAppNamespace,
          conversationKeys: _conversationKeysForLoad(),
          cacheUnderConversationKey: key,
          limit: _transcriptWindowLimit,
        );
        if (!mounted) return;
        final persistedCount = _lines
            .where((l) => l._persisted && !l.system)
            .length;
        if (!force &&
            view.revision > 0 &&
            view.revision <= _paintedTranscriptRevision &&
            view.lines.length <= persistedCount &&
            _lines.any((l) => !l.system)) {
          return;
        }
        _applyTranscriptView(view, force: force);
      } catch (e) {
        AppLog.instance.w("Chat", "syncTranscriptView failed: $e");
      }
    });
    await _transcriptSyncChain;
  }

  void _applyTranscriptView(TranscriptThreadView view, {bool force = false}) {
    final rows = view.lines;
    final key = _conversationKey();
    final hasVisibleLines = _lines.any((l) => !l.system);
    final loaded = rows.map(_lineFromStored).toList();
    setState(() {
      _transcriptHasMore = view.hasMore;
      final optimistic = _outboundLinesPendingOnDisk(rows);
      final sameRoom =
          _transcriptLoadedKey == key || _transcriptLoadedKey == null;
      if (loaded.isNotEmpty || !hasVisibleLines || !sameRoom || force) {
        _lines.removeWhere((l) => l._persisted);
        _lines.addAll(loaded);
      }
      for (final o in optimistic) {
        if (!_lines.any((l) => l.localId == o.localId)) _lines.add(o);
      }
      _dedupeLinesByMessageId();
      _sortLinesByTime();
      if (loaded.isNotEmpty) {
        _transcriptLoadedKey = key;
        _paintedTranscriptRevision = view.revision;
        _emptyTranscriptRetryCount = 0;
        _emptyTranscriptRetry?.cancel();
      }
      _transcriptFlushedLocalIds
        ..clear()
        ..addAll(_lines.where((l) => l._persisted).map((l) => l.localId));
      for (final l in _lines) {
        final mid = l.messageId?.trim() ?? "";
        if (mid.isNotEmpty) _uiSeenMessageIds.add(mid);
      }
    });
    _seedAckStateFromTranscript(rows);
    unawaited(_flushDeliveryPatches());
    _flushBufferedHubDmEvents();
    _scheduleListScroll(force: false);
  }

  _ChatLine _lineFromStored(StoredChatLine s) {
    final d = s.outgoing ? _deliveryFromStored(s) : _MsgDelivery.pending;
    return _ChatLine(
      localId: s.localId,
      text: s.text,
      from: s.from,
      outgoing: s.outgoing,
      messageId: s.messageId,
      delivery: s.outgoing ? d : _MsgDelivery.pending,
      readAckSent: s.readAckSent,
      msgKind: s.msgKind,
      durationMs: s.durationMs,
      audioPath: s.audioPath,
      fileName: s.fileName,
      mimeType: s.mimeType,
      sizeBytes: s.sizeBytes,
      localPath: s.localPath,
      createdAtMs: s.createdAtMs ?? 0,
    );
  }

  Future<List<StoredChatLine>> _loadTranscriptRows() async {
    final ns = _resolvedAppNamespace;
    return ChatTranscriptStore.loadMerged(
      appNamespace: ns,
      conversationKeys: _conversationKeysForLoad(),
      cacheUnderConversationKey: _conversationKey(),
    );
  }

  List<_ChatLine> _outboundLinesPendingOnDisk(List<StoredChatLine> loaded) {
    final loadedMids = <String>{
      for (final r in loaded)
        if (r.messageId?.trim().isNotEmpty ?? false) r.messageId!.trim(),
    };
    final loadedOutboundBubbles = <String>{
      for (final r in loaded.where((r) => r.outgoing))
        "${r.msgKind == "voice" ? "voice|${r.durationMs ?? 0}|${r.localPath ?? r.audioPath ?? ""}" : r.msgKind == "attachment_offer" ? "attachment|${r.fileName ?? ""}|${r.sizeBytes ?? 0}|${r.localPath ?? ""}" : "text|${r.text.trim()}"}|${r.createdAtMs ?? 0}",
    };
    return _lines.where((l) {
      if (!l._persisted || !l.outgoing) return false;
      final mid = l.messageId?.trim() ?? "";
      if (mid.isNotEmpty && loadedMids.contains(mid)) return false;
      final bubble = _outboundBubbleKey(l);
      if (bubble != null && loadedOutboundBubbles.contains(bubble)) {
        return false;
      }
      return true;
    }).toList();
  }

  /// Paint cached transcript synchronously (hub warms cache on contact select).
  bool _paintCachedTranscriptIfAny() {
    final canon = _conversationKey();
    if (canon.isEmpty || canon == "solo") return false;
    // Prefer cache keyed by canonical public_key_hex so we never paint another peer's thread.
    final cached =
        ChatTranscriptStore.peekCachedThread(
          appNamespace: _resolvedAppNamespace,
          conversationKeys: {canon},
        ) ??
        ChatTranscriptStore.peekCachedThread(
          appNamespace: _resolvedAppNamespace,
          conversationKeys: _conversationKeysForLoad(),
        );
    if (cached == null || cached.isEmpty) return false;
    final loaded = cached.map(_lineFromStored).toList();
    _seedAckStateFromTranscript(cached);
    setState(() {
      final optimistic = _outboundLinesPendingOnDisk(cached);
      _lines.removeWhere((l) => l._persisted);
      _lines.addAll(loaded);
      for (final o in optimistic) {
        if (!_lines.any((l) => l.localId == o.localId)) _lines.add(o);
      }
      _dedupeLinesByMessageId();
      _sortLinesByTime();
      _transcriptLoadedKey = _conversationKey();
      _transcriptFlushedLocalIds
        ..clear()
        ..addAll(_lines.where((l) => l._persisted).map((l) => l.localId));
      for (final l in _lines) {
        final mid = l.messageId?.trim() ?? "";
        if (mid.isNotEmpty) _uiSeenMessageIds.add(mid);
      }
    });
    return true;
  }

  void _scheduleSaveTranscript({bool persistAll = false}) {
    // Hub/daemon: :p2p owns transcript I/O on poll — UI writes race the background process.
    if (widget.hubPollsEvents) return;
    unawaited(_enqueueTranscriptFlush(_flushTranscriptIncremental));
    if (persistAll) {
      _scheduleFullTranscriptSave();
      return;
    }
    _saveTranscriptDebounce?.cancel();
    _saveTranscriptDebounce = Timer(const Duration(milliseconds: 1200), () {
      unawaited(_enqueueTranscriptFlush(_flushTranscriptFull));
    });
  }

  void _scheduleFullTranscriptSave() {
    if (widget.hubPollsEvents) return;
    _fullTranscriptSaveTimer?.cancel();
    _fullTranscriptSaveTimer = Timer(const Duration(seconds: 4), () {
      unawaited(_enqueueTranscriptFlush(_flushTranscriptFull));
    });
  }

  Future<void> _enqueueTranscriptFlush(Future<void> Function() fn) {
    final run = _transcriptFlushChain.then((_) => fn());
    _transcriptFlushChain = run.whenComplete(() {});
    return run;
  }

  /// Append only new rows — avoids rewriting 500+ lines on every send (main-thread hang).
  Future<void> _flushTranscriptIncremental() async {
    if (widget.hubPollsEvents) return;
    final ns = _resolvedAppNamespace;
    final key = _conversationKey();
    for (final l in List<_ChatLine>.from(_lines)) {
      if (!l._persisted || _transcriptFlushedLocalIds.contains(l.localId)) {
        continue;
      }
      if (l.outgoing && (l.messageId?.trim().isEmpty ?? true)) continue;
      await ChatTranscriptStore.appendIfNew(
        appNamespace: ns,
        conversationKey: key,
        line: l.toStored(),
      );
      _transcriptFlushedLocalIds.add(l.localId);
    }
  }

  Future<void> _flushTranscriptFull() async {
    if (widget.hubPollsEvents) return;
    _dedupeLinesByMessageId();
    ChatTranscriptStore.invalidateThreadCache(
      appNamespace: _resolvedAppNamespace,
      conversationKeys: _conversationKeysForLoad(),
    );
    final rows = await _loadTranscriptRows();
    _applyDeliveryPatchesFromStoredRows(rows);
    final persisted = List<_ChatLine>.from(
      _lines,
    ).where((l) => l._persisted).map((l) => l.toStored()).toList();
    await ChatTranscriptStore.save(
      appNamespace: _resolvedAppNamespace,
      conversationKey: _conversationKey(),
      lines: persisted,
    );
    _transcriptFlushedLocalIds
      ..clear()
      ..addAll(_lines.where((l) => l._persisted).map((l) => l.localId));
  }

  bool _hasMessageId(String? id) {
    final mid = id?.trim() ?? "";
    if (mid.isEmpty) return false;
    return _lines.any((l) => l.messageId?.trim() == mid);
  }

  void _updateOutgoingDelivery(String refId, _MsgDelivery state) {
    var changed = false;
    for (final l in _lines) {
      if (l.messageId != refId || !l.outgoing) continue;
      if (_deliveryRank(state) > _deliveryRank(l.delivery)) {
        l.delivery = state;
        changed = true;
      }
    }
    if (changed) {
      _pendingDeliveryPatches[refId] = state;
      unawaited(_flushDeliveryPatches());
      if (mounted) setState(() {});
    }
  }

  Future<void> _flushDeliveryPatches() async {
    if (widget.hubPollsEvents) return;
    if (_pendingDeliveryPatches.isEmpty) return;
    final batch = Map<String, _MsgDelivery>.from(_pendingDeliveryPatches);
    _pendingDeliveryPatches.clear();
    final ns = _resolvedAppNamespace;
    final key = _conversationKey();
    for (final e in batch.entries) {
      await ChatTranscriptStore.patchOutgoingDelivery(
        appNamespace: ns,
        conversationKey: key,
        messageId: e.key,
        delivery: e.value.name,
      );
    }
  }

  @override
  void didUpdateWidget(ChatScreen oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.localPeerAlias != widget.localPeerAlias) {
      _refreshInviteUri();
    }
    if (oldWidget.publicKeyHex != widget.publicKeyHex) {
      _refreshInviteUri();
    }
    if (oldWidget.aliasNonce != widget.aliasNonce) {
      _pullAliasFromStore();
    }
    if (oldWidget.appNamespace != widget.appNamespace) {
      _pullAliasFromStore();
      unawaited(_refreshRoomContact());
    }
    final oldKey = _threadKeyForWidget(oldWidget);
    final newKey = _threadKeyForWidget(widget);
    if (oldKey != newKey) {
      final ac = widget.activeContact;
      _roomContact = ac;
      _uiSeenMessageIds.clear();
      _bufferedHubDmEvents.clear();
      _ackedReadIds.clear();
      _transcriptLoadedKey = null;
      _transcriptFlushedLocalIds.clear();
      _reloadGeneration++;
      _emptyTranscriptRetry?.cancel();
      _emptyTranscriptRetryCount = 0;
      _seenLibp2pConnections.clear();
      _paintCachedTranscriptIfAny();
      _remoteInvite = ac?.toConnectInvite();
      if (ac != null) {
        if (ac.hasFullKeys) {
          _stashRemotePublicKey(ac.publicKeyHex);
        }
        if (widget.hubPollsEvents && ac.hasFullKeys) {
          _registerActiveDmPeer();
        }
      } else {
        _remoteInvite = null;
        _learnedRemotePublicKeyHex = null;
        _learnedRemotePublicKeyHex = null;
      }
      _restoreHubLinkState();
      unawaited(_reloadTranscriptForConversation(force: true));
    } else if (oldWidget.activeContact?.publicKeyHex !=
        widget.activeContact?.publicKeyHex) {
      final ac = widget.activeContact;
      if (ac != null && ac.hasFullKeys) {
        _stashRemotePublicKey(ac.publicKeyHex);
        if (widget.hubPollsEvents) {
          _registerActiveDmPeer();
        }
      }
    }
  }

  String get _resolvedAppNamespace {
    final n = widget.appNamespace?.trim();
    if (n != null && n.isNotEmpty) return n;
    return kGhalBolAppNamespace;
  }

  void _pullAliasFromStore() {
    final sig = widget.publicKeyHex?.trim();
    if (!isValidPublicKeyHex(sig)) return;
    IdentityAliasStore.read(
      appNamespace: _resolvedAppNamespace,
      publicKeyHex: sig!,
    ).then((v) {
      if (!mounted) return;
      setState(() => _storeAliasLift = v);
      _refreshInviteUri();
    });
  }

  String _peerLabelForFrom(String from) {
    final f = from.trim();
    final localPk = widget.publicKeyHex?.trim() ?? widget.libp2pPeerId.trim();
    final localWire = libp2pPeerIdFromPublicKeyHex(localPk) ?? "";
    if (localWire.isNotEmpty && _sameLibp2pPeer(f, localWire)) {
      return ghalBolIdName(
        publicKeyHex: widget.publicKeyHex ?? localPk,
        customAlias: _effectiveCustomAlias,
      );
    }
    final ac = widget.activeContact;
    if (ac != null) {
      final a = ac.displayAlias?.trim();
      if (a != null && a.isNotEmpty) return a;
      return ghalBolIdName(publicKeyHex: ac.publicKeyHex);
    }
    if (_joinedRemote &&
        _remoteInvite != null &&
        f == _remoteInvite!.peerId.trim()) {
      final a = _remoteInvite!.peerAlias?.trim();
      if (a != null && a.isNotEmpty) return a;
    }
    if (f.length > 14) {
      return "${f.substring(0, 6)}…${f.substring(f.length - 4)}";
    }
    return f;
  }

  bool _sameLibp2pPeer(String? a, String b) {
    if (a == null) return false;
    return a.trim() == b.trim();
  }

  bool _isOutgoingMessage(_ChatLine l) {
    if (l.system) return false;
    return l.outgoing;
  }

  SavedContact? get _trustContact => _roomContact ?? widget.activeContact;

  void _onContactsStoreChanged() {
    if (!mounted) return;
    unawaited(_refreshRoomContact());
  }

  Future<void> _refreshRoomContact() async {
    final ac = widget.activeContact;
    if (ac == null || !ac.hasPublicKey) {
      if (!mounted) return;
      setState(() => _roomContact = ac);
      return;
    }
    final fresh = await ContactStore.findByPublicKey(
      appNamespace: _resolvedAppNamespace,
      publicKeyHex: ac.publicKeyHex,
    );
    if (!mounted) return;
    setState(() => _roomContact = fresh ?? ac);
  }

  bool _isInboundPeerBlocked(String from, {String? senderPk}) {
    final tc = _trustContact;
    if (tc != null && tc.isBlocked) return true;
    if (senderPk != null && isValidPublicKeyHex(senderPk)) {
      return tc != null &&
          publicKeysEqual(tc.publicKeyHex, senderPk) &&
          tc.isBlocked;
    }
    final f = from.trim();
    if (f.isEmpty) return false;
    final localPk = widget.publicKeyHex?.trim() ?? widget.libp2pPeerId.trim();
    final localWire = libp2pPeerIdFromPublicKeyHex(localPk) ?? "";
    if (localWire.isNotEmpty && _sameLibp2pPeer(f, localWire)) return false;
    return false;
  }

  Future<void> _setContactKnown() async {
    final tc = _trustContact;
    if (tc == null || !tc.hasPublicKey || tc.isKnown) return;
    final pk = tc.publicKeyHex;
    if (!isValidPublicKeyHex(pk)) return;
    await ContactStore.setTrust(
      appNamespace: _resolvedAppNamespace,
      publicKeyHex: pk,
      isKnown: true,
    );
    await _refreshRoomContact();
  }

  Future<void> _setContactBlocked() async {
    final tc = _trustContact;
    if (tc == null || !tc.hasPublicKey) return;
    final pk = tc.publicKeyHex;
    if (!isValidPublicKeyHex(pk)) return;
    await ContactStore.setTrust(
      appNamespace: _resolvedAppNamespace,
      publicKeyHex: pk,
      isBlocked: true,
    );
    await _refreshRoomContact();
  }

  Widget _contactTrustBanner(BuildContext context, GhalBolChatRoomPalette p) {
    final tc = _trustContact;
    if (tc == null) return const SizedBox.shrink();
    if (tc.isBlocked) {
      return Material(
        color: p.isDark ? const Color(0xFF5C3D2E) : const Color(0xFFFFE8E0),
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 10),
          child: Text(
            "You blocked this contact. Unblock from More → Blocked contacts to chat again.",
            style: Theme.of(context).textTheme.bodySmall,
          ),
        ),
      );
    }
    if (!tc.showTrustBanner) return const SizedBox.shrink();
    return Material(
      color: p.isDark ? const Color(0xFF3D4F5C) : const Color(0xFFE8F4FD),
      child: Padding(
        padding: const EdgeInsets.fromLTRB(12, 10, 12, 10),
        child: Row(
          children: [
            Expanded(
              child: Text(
                "You have not added this person yet.",
                style: Theme.of(context).textTheme.bodySmall,
              ),
            ),
            TextButton(
              onPressed: () => unawaited(_setContactKnown()),
              child: const Text("Add"),
            ),
            TextButton(
              onPressed: () => unawaited(_setContactBlocked()),
              child: const Text("Block"),
            ),
          ],
        ),
      ),
    );
  }

  String _bubbleSenderCaption(_ChatLine l) {
    if (_isOutgoingMessage(l)) return "You";
    return _peerLabelForFrom(l.from ?? "");
  }

  _ChatLine _systemLine(String text) => _ChatLine(
    localId: _newLocalId(),
    text: text,
    system: true,
    createdAtMs: DateTime.now().millisecondsSinceEpoch,
  );

  Future<void> _reloadTranscriptForConversation({bool force = false}) async {
    final key = _conversationKey();
    if (widget.hubPollsEvents &&
        key == "solo" &&
        _transcriptLoadedKey != null &&
        _transcriptLoadedKey != "solo") {
      return;
    }
    if (!force && _transcriptLoadedKey == key) return;
    if (_loadingTranscript) {
      _transcriptReloadQueued = true;
      _transcriptReloadQueuedForce |= force;
      return;
    }
    _loadingTranscript = true;
    final gen = ++_reloadGeneration;
    if (_transcriptLoadedKey != key) {
      // New conversation: start from the newest page again.
      _transcriptWindowLimit = _transcriptPageSize;
      _transcriptHasMore = false;
    }
    if (force || _transcriptLoadedKey != key) {
      if (force &&
          _transcriptLoadedKey != null &&
          _transcriptLoadedKey != key) {
        setState(() {
          _lines.removeWhere((l) => l._persisted);
          _uiSeenMessageIds.clear();
          _paintedTranscriptRevision = 0;
        });
      }
      _paintCachedTranscriptIfAny();
    }
    try {
      final view = await ChatTranscriptStore.loadThreadView(
        appNamespace: _resolvedAppNamespace,
        conversationKeys: _conversationKeysForLoad(),
        cacheUnderConversationKey: key,
        limit: _transcriptWindowLimit,
      );
      if (!mounted || gen != _reloadGeneration) return;
      _applyTranscriptView(view, force: force || _transcriptLoadedKey != key);
      _syncOpenChatToNative();
      AppLog.instance.flow(
        "Chat",
        "transcript reload conv=$key rows=${view.lines.length} "
            "rev=${view.revision} marked_loaded=${_transcriptLoadedKey == key} force=$force",
      );
      if (view.lines.isEmpty && widget.hubPollsEvents && key != "solo") {
        _scheduleEmptyTranscriptRetry();
      }
    } finally {
      // Only one load runs at a time, so this run always owns the flag — a superseded
      // generation that left it set would pin the room on "Loading conversation…".
      _loadingTranscript = false;
      final queued = _transcriptReloadQueued;
      final queuedForce = _transcriptReloadQueuedForce;
      _transcriptReloadQueued = false;
      _transcriptReloadQueuedForce = false;
      if (mounted) {
        if (queued) {
          unawaited(_reloadTranscriptForConversation(force: queuedForce));
        } else {
          setState(() {});
        }
      }
    }
  }

  Timer? _emptyTranscriptRetry;
  int _emptyTranscriptRetryCount = 0;

  /// Hub cold start: FFI read can return 0 rows before daemon/unlock path is ready.
  void _scheduleEmptyTranscriptRetry() {
    if (_emptyTranscriptRetryCount >= 4) return;
    _emptyTranscriptRetry?.cancel();
    _emptyTranscriptRetry = Timer(const Duration(milliseconds: 800), () {
      if (!mounted) return;
      _emptyTranscriptRetryCount++;
      unawaited(_reloadTranscriptForConversation(force: true));
    });
  }

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted) return;
      _syncOpenChatToNative();
    });
    _scrollController.addListener(_onScrollForOlderPage);
    WidgetsBinding.instance.addObserver(this);
    ContactStore.changeCount.addListener(_onContactsStoreChanged);
    _voicePreviewPlayer.onPositionChanged.listen((pos) {
      if (mounted && _voicePreviewPath != null) {
        setState(() => _voicePreviewPosition = pos);
      }
    });
    _voicePreviewPlayer.onPlayerComplete.listen((_) {
      if (mounted) {
        setState(() {
          _voicePreviewPlaying = false;
          _voicePreviewPosition = Duration.zero;
        });
      }
    });
    widget.onHubChatAttach?.call(this);
    _roomContact = widget.activeContact;
    final ac = widget.activeContact;
    if (ac != null) {
      if (ac.hasFullKeys) {
        _stashRemotePublicKey(ac.publicKeyHex);
      }
      final inv = ac.toConnectInvite();
      if (inv != null) _remoteInvite = inv;
    }
    final hub = _hubThreadPublicKeyHex();
    if (hub != null) {
      _stashRemotePublicKey(hub);
      _paintCachedTranscriptIfAny();
      unawaited(_reloadTranscriptForConversation());
    } else if (widget.activeContact != null) {
      _paintCachedTranscriptIfAny();
      unawaited(_reloadTranscriptForConversation());
    }
    unawaited(_boot());
    final sig = widget.publicKeyHex?.trim();
    if (widget.localPeerAlias == null && isValidPublicKeyHex(sig)) {
      IdentityAliasStore.read(
        appNamespace: _resolvedAppNamespace,
        publicKeyHex: sig!,
      ).then((v) {
        if (!mounted) return;
        setState(() => _storeAliasLift = v);
        _refreshInviteUri();
      });
    }
  }

  Future<void> _boot() async {
    if (!GhalBolP2p.isAvailable) {
      setState(() {
        _lines.add(
          _systemLine(
            "Chat needs the native library. Sync Android JNI libs, then reopen the app.",
          ),
        );
      });
      return;
    }
    if (_joinedRemote && _remoteInvite != null && _remoteInvite!.hasFullKeys) {
      _stashRemotePublicKey(_remoteInvite!.publicKeyHex);
    }
    _registerActiveDmPeer();
    final pk = _recipientPublicKeyHex();
    if (isValidPublicKeyHex(pk) &&
        (widget.hubPeerStreamReady?.call(pk!) ??
            P2pEventBridge.instance.isStreamReady(pk!))) {
      setState(() {
        _seenLibp2pConnections.add(pk!);
        _chatError = null;
      });
    }
  }

  /// Join [inv]: the hub owns the P2P lifecycle; this surface only re-points at the contact.
  Future<void> applyJoinInvitation(GhalBolConnectInvite inv) async {
    setState(() {
      _remoteInvite = inv;
      _chatError = null;
      _seenLibp2pConnections.clear();
    });
    if (inv.hasFullKeys) {
      _applyRemotePeerKeys(inv.publicKeyHex, peerId: inv.peerId);
    }
    widget.onContactJoined?.call(SavedContact.fromInvite(inv));
    await _reloadTranscriptForConversation(force: true);
  }

  void _handleEvent(Map<String, dynamic> ev) {
    final kind = ev["kind"]?.toString();
    if (kind == "dm_message") {
      final from = ev["from"]?.toString() ?? "?";
      final senderPk = contactPublicKeyHexFromEvent(ev);
      if (_isInboundPeerBlocked(from, senderPk: senderPk)) return;
      final msgKind = ev["msg_kind"]?.toString() ?? "";
      final id = ev["id"]?.toString();
      final refId = ev["ref_id"]?.toString();
      if (msgKind == "text") {
        final ac = widget.activeContact;
        if (ac != null) {
          final sigOk =
              isValidPublicKeyHex(senderPk) &&
              ac.hasPublicKey &&
              _hexEq(senderPk, ac.publicKeyHex);
          final wireOk =
              ac.hasPublicKey &&
              libp2pWireMatchesContactPublicKey(
                wirePeerId: from,
                contactPublicKeyHex: ac.publicKeyHex,
              );
          if (!sigOk && !wireOk) return;
        }
        final text = ev["text"]?.toString() ?? "";
        final myPk = widget.publicKeyHex?.trim() ?? "";
        final isMine =
            isValidPublicKeyHex(myPk) &&
            isValidPublicKeyHex(senderPk) &&
            _hexEq(senderPk, myPk);
        if (!isMine && isValidPublicKeyHex(senderPk)) {
          final ac = widget.activeContact;
          if (ac == null ||
              !ac.hasPublicKey ||
              _hexEq(senderPk, ac.publicKeyHex)) {
            _applyRemotePeerKeys(senderPk, peerId: from);
          }
        }
        if (isMine) {
          final mid = id?.trim() ?? "";
          if (mid.isNotEmpty) {
            _uiSeenMessageIds.add(mid);
          }
          return;
        }
        final mid = id?.trim() ?? "";
        final createdAt = _eventCreatedAtMs(ev);
        var added = false;
        setState(() {
          added = _tryAddTextLine(
            from: from,
            text: text,
            outgoing: isMine,
            messageId: mid.isEmpty ? null : mid,
            createdAtMs: createdAt,
          );
          if (added) {
            _dedupeLinesByMessageId();
            _sortLinesByTime();
            _chatError = null;
          }
        });
        if (!added) return;
        _scheduleSaveTranscript();
        // `ack_received` and `ack_read` are sent by native P2P while this chat is foreground.
        _scheduleListScroll(force: false);
        return;
      }
      if (msgKind == "voice") {
        final ac = widget.activeContact;
        if (ac != null) {
          final sigOk =
              isValidPublicKeyHex(senderPk) &&
              ac.hasPublicKey &&
              _hexEq(senderPk, ac.publicKeyHex);
          final wireOk =
              ac.hasPublicKey &&
              libp2pWireMatchesContactPublicKey(
                wirePeerId: from,
                contactPublicKeyHex: ac.publicKeyHex,
              );
          if (!sigOk && !wireOk) return;
        }
        final myPk = widget.publicKeyHex?.trim() ?? "";
        final isMine =
            isValidPublicKeyHex(myPk) &&
            isValidPublicKeyHex(senderPk) &&
            _hexEq(senderPk, myPk);
        if (!isMine && isValidPublicKeyHex(senderPk)) {
          final ac = widget.activeContact;
          if (ac == null ||
              !ac.hasPublicKey ||
              _hexEq(senderPk, ac.publicKeyHex)) {
            _applyRemotePeerKeys(senderPk, peerId: from);
          }
        }
        if (isMine) {
          final mid = id?.trim() ?? "";
          if (mid.isNotEmpty) {
            _uiSeenMessageIds.add(mid);
          }
          return;
        }
        final mid = id?.trim() ?? "";
        final createdAt = _eventCreatedAtMs(ev);
        var added = false;
        setState(() {
          added = _tryAddVoiceLine(
            from: from,
            outgoing: isMine,
            messageId: mid.isEmpty ? null : mid,
            createdAtMs: createdAt,
            durationMs: _eventInt(ev, "duration_ms"),
            audioPath: ev["audio_path"]?.toString(),
            localPath: ev["local_path"]?.toString(),
            fileName: ev["file_name"]?.toString(),
            mimeType: ev["mime_type"]?.toString(),
            sizeBytes: _eventInt(ev, "size_bytes"),
          );
          if (added) {
            _dedupeLinesByMessageId();
            _sortLinesByTime();
            _chatError = null;
          }
        });
        if (!added) return;
        _scheduleSaveTranscript();
        _scheduleListScroll(force: false);
        return;
      }
      if (msgKind == "attachment_offer") {
        final myPk = widget.publicKeyHex?.trim() ?? "";
        final isMine =
            isValidPublicKeyHex(myPk) &&
            isValidPublicKeyHex(senderPk) &&
            _hexEq(senderPk, myPk);
        if (!isMine && isValidPublicKeyHex(senderPk)) {
          final ac = widget.activeContact;
          if (ac == null ||
              !ac.hasPublicKey ||
              _hexEq(senderPk, ac.publicKeyHex)) {
            _applyRemotePeerKeys(senderPk, peerId: from);
          }
        }
        if (isMine) {
          final mid = id?.trim() ?? "";
          if (mid.isNotEmpty) _uiSeenMessageIds.add(mid);
          return;
        }
        final mid = id?.trim() ?? "";
        final createdAt = _eventCreatedAtMs(ev);
        var added = false;
        setState(() {
          added = _tryAddAttachmentLine(
            from: from,
            outgoing: isMine,
            messageId: mid.isEmpty ? null : mid,
            createdAtMs: createdAt,
            text: ev["text"]?.toString(),
            fileName: ev["file_name"]?.toString(),
            mimeType: ev["mime_type"]?.toString(),
            sizeBytes: _eventInt(ev, "size_bytes"),
            localPath: ev["local_path"]?.toString(),
          );
          if (added) {
            _dedupeLinesByMessageId();
            _sortLinesByTime();
            _chatError = null;
          }
        });
        if (!added) return;
        _scheduleSaveTranscript();
        _scheduleListScroll(force: false);
        return;
      }
      if ((msgKind == kRecipientAckDelivered || msgKind == kRecipientAckRead) &&
          refId != null &&
          refId.isNotEmpty) {
        if (!_inboundAckFromActivePeer(ev)) return;
        if (ev["stores_updated"] != true) return;
        _scheduleTranscriptSync();
        return;
      }
      if (msgKind == kSenderConfirmedReadReceipt &&
          refId != null &&
          refId.isNotEmpty) {
        if (!_inboundAckFromActivePeer(ev)) return;
        if (_hasInboundMessageId(refId)) {
          _markInboundReadAckConfirmed(refId);
        } else {
          _pendingDeliveredAckRefs.add(refId);
        }
        return;
      }
      return;
    }
    if (kind == "node_ready") {
      _refreshInviteUri();
      return;
    }
    if (kind == "peer_identified") {
      final pk = contactPublicKeyHexFromEvent(ev);
      if (isValidPublicKeyHex(pk)) {
        final ac = widget.activeContact;
        if (_peerIdentifiedMatchesContact(pk, ac)) {
          _applyRemotePeerKeys(pk, peerId: libp2pWirePeerIdFromEvent(ev));
          final wire = libp2pWirePeerIdFromEvent(ev);
          if (wire.isNotEmpty && _peerMatchesActive(wire)) {
            setState(() => _chatError = null);
          }
        }
        unawaited(
          ContactStore.mergeDiscoveredContact(
            appNamespace: _resolvedAppNamespace,
            publicKeyHex: pk,
          ),
        );
      }
      return;
    }
    if (kind == "chat_ready") {
      final p = contactKeyFromEvent(ev);
      if (p.isNotEmpty && _peerMatchesActive(p)) {
        setState(() => _chatError = null);
        _syncOpenChatToNative();
      }
      return;
    }
    if (kind == "peer_connected") {
      final p = contactKeyFromEvent(ev);
      if (p.isEmpty) return;
      if (!_seenLibp2pConnections.contains(p)) {
        _seenLibp2pConnections.add(p);
      }
      if (_peerMatchesActive(p)) {
        setState(() => _chatError = null);
        if (!_seenLibp2pConnections.contains(p)) {
          _seenLibp2pConnections.add(p);
        }
        unawaited(_registerActiveDmPeer());
        _syncOpenChatToNative();
      }
      return;
    }
    if (kind == "outbound_sent") {
      if (mounted) setState(() => _chatError = null);
      if (widget.hubPollsEvents && ev["stores_updated"] == true) {
        _scheduleTranscriptSync(force: true);
      }
      return;
    }
    if (kind == "send_failed") {
      final id = ev["message_id"]?.toString() ?? "";
      final err = ev["error"]?.toString() ?? "send failed";
      final hint = networkAwareUserP2pError(err);
      AppLog.instance.w(
        "Chat",
        "send_failed id=$id err=$err ui=${hint ?? "(hidden)"}",
      );
      setState(() {
        _chatError = hint;
        if (id.isNotEmpty) {
          for (final l in _lines) {
            if (l.messageId != id) continue;
            l.delivery = _MsgDelivery.pending;
          }
        }
      });
      _syncOpenChatToNative();
      _scheduleSaveTranscript();
      return;
    }
    if (kind == "peer_disconnected") {
      final p = contactKeyFromEvent(ev);
      if (_peerMatchesActive(p)) {
        _onActivePeerLinkLost();
        if (mounted) setState(() {});
      }
      return;
    }
    if (kind == "dial_failed") {
      final err = ev["error"]?.toString() ?? "?";
      if (isTransientP2pLinkError(err)) {
        AppLog.instance.flow("Chat", "dial_failed (transient): $err");
        return;
      }
      final hint = networkAwareUserP2pError(err);
      if (hint == null) return;
      AppLog.instance.w("Chat", "dial_failed: $err");
      setState(() => _chatError = hint);
    }
  }

  Future<void> copyInvitationLink() async {
    final uri = await _resolveInviteUriForShare();
    if (uri == null) return;
    await Clipboard.setData(ClipboardData(text: uri));
    if (!mounted) return;
    ScaffoldMessenger.of(
      context,
    ).showSnackBar(const SnackBar(content: Text("Invitation copied.")));
  }

  Future<void> shareInvitationLink() async {
    final uri = await _resolveInviteUriForShare();
    if (uri == null) return;
    await SharePlus.instance.share(
      ShareParams(text: uri, subject: "Ghal Bol invitation"),
    );
  }

  /// Same source as QR screen: native store alias + public key.
  Future<String?> _resolveInviteUriForShare() async {
    final wire = resolvePublicKeyHex(storedHex: widget.publicKeyHex);
    if (wire == null || !isValidPublicKeyHex(wire)) return null;
    final alias =
        _effectiveCustomAlias ??
        await IdentityAliasStore.read(
          appNamespace: _resolvedAppNamespace,
          publicKeyHex: wire,
        );
    return buildGhalBolInviteUri(
      publicKeyHex: wire,
      identityWire: wire,
      peerAlias: alias,
    );
  }

  void _refreshInviteUri() {
    if (mounted) setState(() {});
  }

  Future<bool> _deliverOutgoingLine(_ChatLine line) async {
    if (line.delivery == _MsgDelivery.delivered ||
        line.delivery == _MsgDelivery.read ||
        line.delivery == _MsgDelivery.downloaded) {
      return true;
    }
    final recipientRaw = _recipientPublicKeyHex()?.trim();
    if (!isValidPublicKeyHex(recipientRaw)) {
      if (!mounted) return false;
      setState(() {
        line.delivery = _MsgDelivery.failed;
        _chatError = "Connecting…";
      });
      _scheduleSaveTranscript();
      return false;
    }
    if (!mounted) return false;
    final recipient = recipientRaw!;
    final myPk = widget.publicKeyHex?.trim().toLowerCase() ?? "";
    if (myPk.isNotEmpty && recipient.toLowerCase() == myPk) {
      if (!mounted) return false;
      setState(() {
        line.delivery = _MsgDelivery.failed;
        _chatError = "This is your own device.";
      });
      _scheduleSaveTranscript();
      return false;
    }
    // Native enqueue is non-blocking; outbox retries until the peer is connected.
    final recipientShort = recipient.length > 16
        ? "${recipient.substring(0, 8)}…${recipient.substring(recipient.length - 8)}"
        : recipient;
    final Map<String, dynamic> r;
    if (line.isVoice) {
      AppLog.instance.flowJson("Chat", "send_voice_dm enqueue", {
        "recipient_pk": recipientShort,
        "duration_ms": line.durationMs,
        "local_id": line.localId,
        "message_id": line.messageId,
      });
      r = await GhalBolP2p.sendVoiceDm(
        recipientPublicKeyHex: recipient,
        durationMs: line.durationMs ?? 0,
        pcmPath: line.localPath,
      );
    } else if (line.isAttachment) {
      AppLog.instance.flowJson("Chat", "send_attachment enqueue", {
        "recipient_pk": recipientShort,
        "file_name": line.fileName,
        "size_bytes": line.sizeBytes,
        "local_id": line.localId,
        "message_id": line.messageId,
      });
      r = await GhalBolP2p.sendAttachment(recipient, {
        "file_path": line.localPath,
        "file_name": line.fileName,
        "mime_type": line.mimeType,
      });
    } else {
      AppLog.instance.flowJson("Chat", "send_text_dm enqueue", {
        "recipient_pk": recipientShort,
        "text_len": line.text.length,
        "local_id": line.localId,
        "message_id": line.messageId,
      });
      r = await GhalBolP2p.sendTextDm(recipient, line.text);
    }
    if (!mounted) return false;
    if (r["ok"] != true) {
      AppLog.instance.w(
        "Chat",
        "send_${line.isVoice ? "voice" : line.isAttachment ? "attachment" : "text"}_dm failed: ${r["error"]}",
      );
      final err = r["error"]?.toString() ?? "send failed";
      final mid = line.messageId?.trim() ?? "";
      final hint = networkAwareUserP2pError(err);
      AppLog.instance.w(
        "Chat",
        "send_${line.isVoice ? "voice" : line.isAttachment ? "attachment" : "text"}_dm: $err ui=${hint ?? "(hidden)"}",
      );
      setState(() {
        line.delivery = _MsgDelivery.pending;
        _chatError = hint;
      });
      if (!line.isVoice && !line.isAttachment && mid.isNotEmpty && GhalBolP2p.isRequeueAvailable) {
        await GhalBolP2p.requeueOutboundDm(
          messageId: mid,
          recipientPublicKeyHex: recipient,
          text: line.text,
        );
      }
      _syncOpenChatToNative();
      _scheduleSaveTranscript();
      return false;
    }
    final assignedId = r["message_id"]?.toString();
    AppLog.instance.flow(
      "Chat",
      "send_${line.isVoice ? "voice" : line.isAttachment ? "attachment" : "text"}_dm queued msg_id=$assignedId",
    );
    setState(() {
      line.messageId = assignedId;
      _chatError = null;
    });
    _scheduleSaveTranscript();
    if (widget.hubPollsEvents) {
      ChatTranscriptStore.invalidateThreadCache(
        appNamespace: _resolvedAppNamespace,
        conversationKeys: _conversationKeysForLoad(),
      );
      P2pEventBridge.instance.drainNow();
      unawaited(syncTranscriptView(force: true));
    }
    return true;
  }

  void _send() {
    final t = _msgCtrl.text.trim();
    if (t.isEmpty) return;
    if (_trustContact?.isBlocked == true) return;
    unawaited(_sendOutboundText(t));
  }

  Future<void> _sendOutboundText(String t) async {
    await _setContactKnown();
    if (!mounted) return;
    if (_trustContact?.isBlocked == true) return;

    _msgCtrl.clear();

    final line = _ChatLine(
      localId: _newLocalId(),
      from: widget.publicKeyHex ?? widget.libp2pPeerId,
      text: t,
      outgoing: true,
      delivery: _MsgDelivery.pending,
      createdAtMs: DateTime.now().millisecondsSinceEpoch,
    );
    setState(() => _lines.add(line));
    _scheduleListScroll(force: true);
    _scheduleSaveTranscript();

    final recipient = _recipientPublicKeyHex()?.trim();
    if (isValidPublicKeyHex(recipient)) {
      unawaited(
        ContactStore.touchChatPreview(
          appNamespace: _resolvedAppNamespace,
          contactPublicKeyHex: recipient!,
          preview: t,
          messageAtMs: line.createdAtMs,
        ),
      );
    }
    await _deliverOutgoingLine(line);
  }

  /// Tap the mic to begin recording. Recording continues until the user taps
  /// stop (→ preview) or cancel (→ discard); it is never auto-sent.
  Future<void> _startVoiceRecording() async {
    if (_isRecordingVoice ||
        _voicePreviewPath != null ||
        _trustContact?.isBlocked == true) {
      return;
    }
    // permission_handler has no Linux implementation; on mobile we still request
    // explicitly, on desktop we rely on `record`'s own permission probe.
    if (Platform.isAndroid || Platform.isIOS) {
      final status = await Permission.microphone.request();
      if (!mounted) return;
      if (!status.isGranted) {
        ScaffoldMessenger.of(context).showSnackBar(
          const SnackBar(
            content: Text(
              "Microphone permission is required to record voice messages.",
            ),
          ),
        );
        return;
      }
    }
    bool hasPerm;
    try {
      hasPerm = await _voiceRecorder.hasPermission();
    } catch (e) {
      if (!mounted) return;
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text("Microphone unavailable: $e")),
      );
      return;
    }
    if (!hasPerm) {
      if (!mounted) return;
      ScaffoldMessenger.of(context).showSnackBar(
        const SnackBar(
          content: Text(
            "Microphone permission is required to record voice messages.",
          ),
        ),
      );
      return;
    }
    final tmp = await getTemporaryDirectory();
    final path =
        "${tmp.path}/ghalbol_voice_${DateTime.now().microsecondsSinceEpoch}.wav";
    try {
      await _voiceRecorder.start(
        const RecordConfig(
          encoder: AudioEncoder.wav,
          sampleRate: 48000,
          numChannels: 1,
        ),
        path: path,
      );
    } catch (e) {
      if (!mounted) return;
      ScaffoldMessenger.of(
        context,
      ).showSnackBar(SnackBar(content: Text("Could not start recording: $e")));
      return;
    }
    if (!mounted) return;
    setState(() {
      _isRecordingVoice = true;
      _recordingStartedAt = DateTime.now();
      _recordingElapsed = Duration.zero;
      _recordingPath = path;
      for (var i = 0; i < _liveVoiceLevels.length; i++) {
        _liveVoiceLevels[i] = 0.12;
      }
    });
    await _voiceAmplitudeSub?.cancel();
    _voiceAmplitudeSub = _voiceRecorder
        .onAmplitudeChanged(const Duration(milliseconds: 80))
        .listen((amp) {
      if (!mounted || !_isRecordingVoice) return;
      // Amplitude.current is dBFS (typically -160..0). Map into a lively 0..1.
      final normalized = ((amp.current + 45) / 45).clamp(0.08, 1.0);
      setState(() {
        _liveVoiceLevels.removeAt(0);
        _liveVoiceLevels.add(normalized);
      });
    });
    _recordingTimer?.cancel();
    _recordingTimer = Timer.periodic(const Duration(milliseconds: 200), (_) {
      final started = _recordingStartedAt;
      if (!mounted || started == null) return;
      final elapsed = DateTime.now().difference(started);
      setState(() => _recordingElapsed = elapsed);
    });
    _recordingAutoStopTimer?.cancel();
    _recordingAutoStopTimer = Timer(
      const Duration(milliseconds: _maxVoiceDurationMs),
      () {
        // Reached the 2-min cap: stop into preview (still requires confirm).
        unawaited(_stopVoiceRecordingToPreview());
      },
    );
  }

  /// Cancel while recording — discard the clip, back to the normal composer.
  Future<void> _cancelVoiceRecording() async {
    if (!_isRecordingVoice) return;
    final path = await _finishRecorder();
    if (!mounted) return;
    setState(() => _isRecordingVoice = false);
    _deleteVoiceFile(path);
  }

  /// Stop while recording — keep the clip and enter the preview state.
  Future<void> _stopVoiceRecordingToPreview() async {
    if (!_isRecordingVoice) return;
    final started = _recordingStartedAt;
    final elapsed = started == null
        ? _recordingElapsed
        : DateTime.now().difference(started);
    final path = await _finishRecorder();
    if (!mounted) return;
    final filePath = (path ?? "").trim();
    final durationMs = elapsed.inMilliseconds
        .clamp(0, _maxVoiceDurationMs)
        .toInt();
    if (filePath.isEmpty || durationMs < _minVoiceDurationMs) {
      setState(() => _isRecordingVoice = false);
      _deleteVoiceFile(filePath);
      if (mounted && filePath.isNotEmpty) {
        ScaffoldMessenger.of(context).showSnackBar(
          const SnackBar(content: Text("Recording too short")),
        );
      }
      return;
    }
    setState(() {
      _isRecordingVoice = false;
      _voicePreviewPath = filePath;
      _voicePreviewDurationMs = durationMs;
      _voicePreviewPlaying = false;
      _voicePreviewPosition = Duration.zero;
      // Freeze the live mic bars into the preview waveform when available.
      final captured = List<double>.from(_liveVoiceLevels);
      final hasSignal = captured.any((v) => v > 0.2);
      _voicePreviewLevels = hasSignal
          ? captured
          : _VoiceWaveform.synthLevels(
              _VoiceWaveform.seedFrom(filePath, null, durationMs),
              count: 40,
            );
    });
  }

  /// Stops the recorder and its timers, returning the captured file path.
  Future<String?> _finishRecorder() async {
    _recordingTimer?.cancel();
    _recordingAutoStopTimer?.cancel();
    await _voiceAmplitudeSub?.cancel();
    _voiceAmplitudeSub = null;
    String? path;
    try {
      path = await _voiceRecorder.stop();
    } catch (_) {
      path = _recordingPath;
    }
    _recordingStartedAt = null;
    _recordingElapsed = Duration.zero;
    _recordingPath = null;
    return path ?? _recordingPath;
  }

  void _deleteVoiceFile(String? path) {
    final p = (path ?? "").trim();
    if (p.isEmpty) return;
    unawaited(File(p).delete().catchError((_) => File(p)));
  }

  Future<void> _toggleVoicePreviewPlayback() async {
    final path = _voicePreviewPath;
    if (path == null) return;
    if (_voicePreviewPlaying) {
      await _voicePreviewPlayer.pause();
      if (mounted) setState(() => _voicePreviewPlaying = false);
      return;
    }
    try {
      if (_voicePreviewPosition == Duration.zero) {
        await _voicePreviewPlayer.play(DeviceFileSource(path));
      } else {
        await _voicePreviewPlayer.resume();
      }
    } catch (_) {
      return;
    }
    if (mounted) setState(() => _voicePreviewPlaying = true);
  }

  Future<void> _seekVoicePreview(double fraction) async {
    final total = _voicePreviewDurationMs;
    if (total <= 0) return;
    final target = Duration(milliseconds: (total * fraction).round());
    try {
      await _voicePreviewPlayer.seek(target);
    } catch (_) {
      return;
    }
    if (mounted) setState(() => _voicePreviewPosition = target);
  }

  /// Trash the previewed clip without sending.
  Future<void> _discardVoicePreview() async {
    final path = _voicePreviewPath;
    await _voicePreviewPlayer.stop();
    if (!mounted) return;
    setState(() {
      _voicePreviewPath = null;
      _voicePreviewDurationMs = 0;
      _voicePreviewPlaying = false;
      _voicePreviewPosition = Duration.zero;
      _voicePreviewLevels = const [];
    });
    _deleteVoiceFile(path);
  }

  /// Send the previewed clip and return to the normal composer.
  Future<void> _sendVoicePreview() async {
    final path = _voicePreviewPath;
    final durationMs = _voicePreviewDurationMs;
    if (path == null || _voiceSending) return;
    setState(() => _voiceSending = true);
    await _voicePreviewPlayer.stop();
    if (!mounted) return;
    setState(() {
      _voicePreviewPath = null;
      _voicePreviewDurationMs = 0;
      _voicePreviewPlaying = false;
      _voicePreviewPosition = Duration.zero;
      _voicePreviewLevels = const [];
    });
    await _sendOutboundVoice(path, durationMs);
    if (mounted) setState(() => _voiceSending = false);
  }

  Future<void> _sendOutboundVoice(String pcmPath, int durationMs) async {
    await _setContactKnown();
    if (!mounted) return;
    if (_trustContact?.isBlocked == true) return;
    final line = _ChatLine(
      localId: _newLocalId(),
      from: widget.publicKeyHex ?? widget.libp2pPeerId,
      text: "",
      outgoing: true,
      delivery: _MsgDelivery.pending,
      msgKind: "voice",
      durationMs: durationMs,
      localPath: pcmPath,
      fileName: pcmPath.split(Platform.pathSeparator).last,
      mimeType: "audio/wav",
      createdAtMs: DateTime.now().millisecondsSinceEpoch,
    );
    setState(() => _lines.add(line));
    _scheduleListScroll(force: true);
    _scheduleSaveTranscript();
    final recipient = _recipientPublicKeyHex()?.trim();
    if (isValidPublicKeyHex(recipient)) {
      unawaited(
        ContactStore.touchChatPreview(
          appNamespace: _resolvedAppNamespace,
          contactPublicKeyHex: recipient!,
          preview: "Voice message",
          messageAtMs: line.createdAtMs,
        ),
      );
    }
    await _deliverOutgoingLine(line);
  }

  Future<void> _pickAndSendAttachment() async {
    if (_trustContact?.isBlocked == true) return;
    final file = await FilePicker.pickFile();
    if (!mounted || file == null) return;
    final path = file.path?.trim() ?? "";
    if (path.isEmpty) return;
    await _sendOutboundAttachment(
      filePath: path,
      fileName: file.name,
      sizeBytes: file.size,
    );
  }

  Future<void> _sendOutboundAttachment({
    required String filePath,
    required String fileName,
    required int sizeBytes,
  }) async {
    await _setContactKnown();
    if (!mounted) return;
    if (_trustContact?.isBlocked == true) return;
    final preview = "📎 $fileName";
    final line = _ChatLine(
      localId: _newLocalId(),
      from: widget.publicKeyHex ?? widget.libp2pPeerId,
      text: preview,
      outgoing: true,
      delivery: _MsgDelivery.pending,
      msgKind: "attachment_offer",
      fileName: fileName,
      mimeType: "application/octet-stream",
      sizeBytes: sizeBytes,
      localPath: filePath,
      createdAtMs: DateTime.now().millisecondsSinceEpoch,
    );
    setState(() => _lines.add(line));
    _scheduleListScroll(force: true);
    _scheduleSaveTranscript();
    final recipient = _recipientPublicKeyHex()?.trim();
    if (isValidPublicKeyHex(recipient)) {
      unawaited(
        ContactStore.touchChatPreview(
          appNamespace: _resolvedAppNamespace,
          contactPublicKeyHex: recipient!,
          preview: preview,
          messageAtMs: line.createdAtMs,
        ),
      );
    }
    await _deliverOutgoingLine(line);
  }

  static String _formatBytes(int? bytes) {
    final n = bytes ?? 0;
    if (n <= 0) return "Unknown size";
    if (n < 1024) return "$n B";
    if (n < 1024 * 1024) return "${(n / 1024).toStringAsFixed(1)} KB";
    return "${(n / (1024 * 1024)).toStringAsFixed(1)} MB";
  }

  Widget _attachmentBubble(_ChatLine line, Color foreground, Color metaColor) {
    final name = (line.fileName?.trim().isNotEmpty == true)
        ? line.fileName!.trim()
        : line.text.replaceFirst("📎", "").trim();
    final hasLocal = (line.localPath?.trim().isNotEmpty ?? false);
    final offerId = line.messageId?.trim() ?? "";
    final downloading =
        !hasLocal && offerId.isNotEmpty && _downloadingOfferIds.contains(offerId);
    if (hasLocal && offerId.isNotEmpty) _downloadingOfferIds.remove(offerId);
    final isImage = _looksLikeImageAttachment(name, line.mimeType);
    return ConstrainedBox(
      constraints: const BoxConstraints(minWidth: 220, maxWidth: 320),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        mainAxisSize: MainAxisSize.min,
        children: [
          if (hasLocal && isImage) ...[
            GestureDetector(
              onTap: () => unawaited(_openAttachment(line)),
              child: ClipRRect(
                borderRadius: BorderRadius.circular(8),
                child: Image.file(
                  File(line.localPath!.trim()),
                  height: 160,
                  width: double.infinity,
                  fit: BoxFit.cover,
                  errorBuilder: (_, _, _) => const SizedBox.shrink(),
                ),
              ),
            ),
            const SizedBox(height: 8),
          ],
          Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              Icon(
                isImage
                    ? Icons.image_outlined
                    : Icons.insert_drive_file_outlined,
                color: foreground,
                size: 30,
              ),
              const SizedBox(width: 10),
              Flexible(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      name.isEmpty ? "Attachment" : name,
                      style: TextStyle(
                        color: foreground,
                        fontWeight: FontWeight.w700,
                        fontSize: 14,
                      ),
                      maxLines: 2,
                      overflow: TextOverflow.ellipsis,
                    ),
                    const SizedBox(height: 2),
                    Text(
                      hasLocal
                          ? "Downloaded"
                          : downloading
                          ? "Downloading…"
                          : _formatBytes(line.sizeBytes),
                      style: TextStyle(color: metaColor, fontSize: 12),
                    ),
                  ],
                ),
              ),
              if (!line.outgoing && !hasLocal) ...[
                const SizedBox(width: 8),
                if (downloading)
                  SizedBox(
                    width: 20,
                    height: 20,
                    child: CircularProgressIndicator(
                      strokeWidth: 2,
                      color: foreground,
                    ),
                  )
                else
                  IconButton(
                    tooltip: "Download",
                    icon: Icon(Icons.download_rounded, color: foreground),
                    onPressed: () => unawaited(_downloadAttachment(line)),
                  ),
              ],
              if (hasLocal) ...[
                const SizedBox(width: 4),
                IconButton(
                  tooltip: "Save",
                  icon: Icon(Icons.save_alt_rounded, color: foreground),
                  onPressed: () => unawaited(_saveAttachment(line)),
                ),
                IconButton(
                  tooltip: "Open",
                  icon: Icon(Icons.open_in_new_rounded, color: foreground),
                  onPressed: () => unawaited(_openAttachment(line)),
                ),
              ],
            ],
          ),
        ],
      ),
    );
  }

  static bool _looksLikeImageAttachment(String name, String? mime) {
    final m = (mime ?? "").toLowerCase();
    if (m.startsWith("image/")) return true;
    final lower = name.toLowerCase();
    return lower.endsWith(".jpg") ||
        lower.endsWith(".jpeg") ||
        lower.endsWith(".png") ||
        lower.endsWith(".gif") ||
        lower.endsWith(".webp") ||
        lower.endsWith(".avif") ||
        lower.endsWith(".heic") ||
        lower.endsWith(".bmp");
  }

  Future<void> _saveAttachment(_ChatLine line) async {
    final path = line.localPath?.trim() ?? "";
    if (path.isEmpty) return;
    final src = File(path);
    if (!await src.exists()) {
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          const SnackBar(content: Text("Attachment file is missing")),
        );
      }
      return;
    }
    final name = (line.fileName?.trim().isNotEmpty == true)
        ? line.fileName!.trim()
        : line.text.replaceFirst("📎", "").trim();
    final safeName = name.isEmpty ? "attachment" : name;
    try {
      final bytes = await src.readAsBytes();
      final saved = await FilePicker.saveFile(
        dialogTitle: "Save attachment",
        fileName: safeName,
        bytes: Uint8List.fromList(bytes),
      );
      if (!mounted || saved == null) return;
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text("Saved to $saved")),
      );
    } catch (e) {
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(content: Text("Could not save attachment: $e")),
        );
      }
    }
  }

  Future<void> _openAttachment(_ChatLine line) async {
    final path = line.localPath?.trim() ?? "";
    if (path.isEmpty) return;
    final name = (line.fileName?.trim().isNotEmpty == true)
        ? line.fileName!.trim()
        : line.text.replaceFirst("📎", "").trim();
    // Android/iOS block handing raw file:// URIs to other apps. Show images
    // in-app; for other types use the share sheet (FileProvider-backed).
    if (_looksLikeImageAttachment(name, line.mimeType)) {
      if (!mounted) return;
      await Navigator.of(context).push(
        MaterialPageRoute<void>(
          fullscreenDialog: true,
          builder: (_) => _AttachmentImageViewer(
            path: path,
            title: name,
            onSave: () => _saveAttachment(line),
          ),
        ),
      );
      return;
    }
    if (Platform.isAndroid || Platform.isIOS) {
      try {
        await SharePlus.instance.share(
          ShareParams(
            files: [XFile(path, mimeType: line.mimeType)],
            subject: name.isEmpty ? "Attachment" : name,
          ),
        );
      } catch (e) {
        if (mounted) {
          ScaffoldMessenger.of(context).showSnackBar(
            SnackBar(content: Text("Could not open attachment: $e")),
          );
        }
      }
      return;
    }
    final ok = await launchUrl(
      Uri.file(path),
      mode: LaunchMode.externalApplication,
    );
    if (!ok && mounted) {
      ScaffoldMessenger.of(context).showSnackBar(
        const SnackBar(content: Text("Could not open attachment")),
      );
    }
  }

  Future<void> _downloadAttachment(_ChatLine line) async {
    final peer = _recipientPublicKeyHex()?.trim();
    final offerId = line.messageId?.trim() ?? "";
    if (!isValidPublicKeyHex(peer) || offerId.isEmpty) return;
    if (!_downloadingOfferIds.add(offerId)) return;
    setState(() {});
    final r = await GhalBolP2p.attachmentFetch({
      "peer_public_key_hex": peer,
      "offer_id": offerId,
    });
    if (!mounted) return;
    // Native reports `downloading` while bytes are still on the wire; the transcript
    // patch plus the `attachment_complete` poll event finish the bubble.
    final pending = r["downloading"] == true;
    if (r["ok"] == true) {
      if (!pending) _downloadingOfferIds.remove(offerId);
      await syncTranscriptView(force: true);
      if (mounted) setState(() {});
      return;
    }
    _downloadingOfferIds.remove(offerId);
    setState(() {});
    ScaffoldMessenger.of(context).showSnackBar(
      SnackBar(content: Text(r["error"]?.toString() ?? "Download failed")),
    );
  }

  Widget _deliveryTicks(_ChatLine line, Color color, {Color? failedColor}) {
    if (!line.outgoing) return const SizedBox.shrink();
    final d = line.delivery;
    if (d == _MsgDelivery.pending) {
      return Icon(
        Icons.schedule,
        size: 14,
        color: color.withValues(alpha: 0.75),
      );
    }
    if (d == _MsgDelivery.failed) {
      return Icon(
        Icons.error_outline,
        size: 14,
        color: failedColor ?? Colors.orange.shade700,
      );
    }
    if (d == _MsgDelivery.sent) {
      return Icon(Icons.done, size: 14, color: color);
    }
    if (d == _MsgDelivery.delivered) {
      return Icon(Icons.done_all, size: 14, color: color);
    }
    if (d == _MsgDelivery.downloaded) {
      return Icon(Icons.file_download_done, size: 14, color: color);
    }
    // Read (double check, blue).
    return Icon(
      Icons.done_all,
      size: 14,
      color: Colors.lightBlueAccent.shade200,
    );
  }

  Future<String?> _pasteInviteDialog() async {
    final ctrl = TextEditingController();
    return showDialog<String>(
      context: context,
      builder: (ctx) => AlertDialog(
        title: const Text("Paste invitation"),
        content: TextField(
          controller: ctrl,
          autofocus: true,
          maxLines: 5,
          decoration: const InputDecoration(
            hintText: "https://ghalbol.com/connect/…",
            border: OutlineInputBorder(),
          ),
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(ctx),
            child: const Text("Cancel"),
          ),
          FilledButton(
            onPressed: () => Navigator.pop(ctx, ctrl.text.trim()),
            child: const Text("Continue"),
          ),
        ],
      ),
    );
  }

  Future<void> _openJoinFlow() async {
    if (!GhalBolP2p.isAvailable) return;
    String? got;
    if (kIsWeb) {
      got = await _pasteInviteDialog();
    } else if (Platform.isAndroid || Platform.isIOS) {
      final mode = await showModalBottomSheet<String>(
        context: context,
        showDragHandle: true,
        builder: (ctx) => SafeArea(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              ListTile(
                leading: const Icon(Icons.qr_code_scanner),
                title: const Text("Scan QR code"),
                onTap: () => Navigator.pop(ctx, "scan"),
              ),
              ListTile(
                leading: const Icon(Icons.link),
                title: const Text("Paste invitation link"),
                onTap: () => Navigator.pop(ctx, "paste"),
              ),
            ],
          ),
        ),
      );
      if (!mounted || mode == null) return;
      if (mode == "paste") {
        got = await _pasteInviteDialog();
      } else {
        got = await Navigator.of(context).push<String>(
          MaterialPageRoute(builder: (_) => const InviteScanScreen()),
        );
      }
    } else {
      got = await _pasteInviteDialog();
    }
    if (!mounted || got == null || got.isEmpty) return;
    final uri = InviteScanScreen.extractInviteUri(got) ?? got.trim();
    final inv = GhalBolConnectInvite.tryParseInviteUri(uri);
    if (inv == null) {
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(
          content: SelectableText(
            GhalBolConnectInvite.explainInviteParseFailure(uri) ??
                "That is not a valid invitation.",
          ),
        ),
      );
      return;
    }
    if (!GhalBolConnectInvite.verifyInvite(inv)) {
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: SelectableText("This invite could not be verified.")),
      );
      return;
    }
    await applyJoinInvitation(inv);
  }

  Future<void> _openShareSheet() async {
    if (!mounted) return;
    final pk = widget.publicKeyHex?.trim() ?? "";
    if (!isValidPublicKeyHex(pk)) return;
    P2pEventBridge.instance.drainNow();
    await Navigator.of(context).push<void>(
      MaterialPageRoute(
        builder: (_) => ShareInviteScreen(
          publicKeyHex: pk,
          appNamespace: _resolvedAppNamespace,
          readListenReady: () => P2pEventBridge.instance.isNodeReady,
          onParentRefresh: _refreshInviteUri,
        ),
      ),
    );
    if (mounted) _refreshInviteUri();
  }

  Widget _connectionStatusBar(BuildContext context) {
    final err = _chatError?.trim().isNotEmpty == true
        ? _chatError!.trim()
        : null;
    if (err == null || err.isEmpty) {
      return const SizedBox.shrink();
    }
    final p = GhalBolChatRoomPalette.of(context);
    return Material(
      color: p.appBarBg,
      child: Container(
        width: double.infinity,
        padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 6),
        decoration: BoxDecoration(
          border: Border(
            bottom: BorderSide(color: p.appBarDivider.withValues(alpha: 0.35)),
          ),
        ),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Expanded(
              child: SelectableText(
                err,
                style: Theme.of(
                  context,
                ).textTheme.bodySmall?.copyWith(color: Colors.orange.shade800),
              ),
            ),
            IconButton(
              tooltip: "Dismiss",
              icon: const Icon(Icons.close, size: 18),
              color: p.metaText,
              onPressed: () => setState(() => _chatError = null),
            ),
          ],
        ),
      ),
    );
  }

  @override
  void dispose() {
    widget.onHubChatDetach?.call(this);
    ContactStore.changeCount.removeListener(_onContactsStoreChanged);
    WidgetsBinding.instance.removeObserver(this);
    _saveTranscriptDebounce?.cancel();
    _fullTranscriptSaveTimer?.cancel();
    _transcriptSyncDebounce?.cancel();
    _emptyTranscriptRetry?.cancel();
    _recordingTimer?.cancel();
    _recordingAutoStopTimer?.cancel();
    unawaited(_voiceAmplitudeSub?.cancel());
    unawaited(_voiceRecorder.dispose());
    unawaited(_voicePreviewPlayer.dispose());
    _deleteVoiceFile(_voicePreviewPath);
    _deleteVoiceFile(_recordingPath);
    unawaited(_flushDeliveryPatches());
    _scrollController.removeListener(_onScrollForOlderPage);
    _scrollController.dispose();
    _msgCtrl.dispose();
    super.dispose();
  }

  static const double _stickToBottomThreshold = 140;

  /// Reversed list: oldest lines sit near [ScrollPosition.maxScrollExtent] (top).
  /// When the user scrolls up to older history and more exists, grow the window.
  void _onScrollForOlderPage() {
    if (!_transcriptHasMore || _loadingOlderPage) return;
    if (!_scrollController.hasClients) return;
    final pos = _scrollController.position;
    if (pos.pixels >= pos.maxScrollExtent - _loadOlderThreshold) {
      unawaited(_loadOlderMessages());
    }
  }

  static const double _loadOlderThreshold = 400;

  /// Grow the transcript window by one page and repaint. `reverse: true` keeps the
  /// user's current position stable while older lines appear above.
  Future<void> _loadOlderMessages() async {
    if (_loadingOlderPage || !_transcriptHasMore) return;
    _loadingOlderPage = true;
    _transcriptWindowLimit += _transcriptPageSize;
    try {
      await syncTranscriptView(force: true);
    } finally {
      _loadingOlderPage = false;
    }
  }

  /// When [force] is true (e.g. user just sent), always scroll. Otherwise only if already near the end.
  ///
  /// Runs twice on successive frames: after [setState], [ListView.builder] may not have updated
  /// [ScrollPosition.maxScrollExtent] yet, so a single [jumpTo]/[animateTo] can stop short of the
  /// new last message.
  void _scheduleListScroll({required bool force}) {
    void scrollToEndOnce() {
      if (!mounted || !_scrollController.hasClients) return;
      final pos = _scrollController.position;
      // [ListView.reverse] == true: newest messages sit at scroll offset 0 (bottom).
      if (!force && pos.pixels > _stickToBottomThreshold) return;
      if (force) {
        _scrollController.jumpTo(0);
      } else {
        _scrollController.animateTo(
          0,
          duration: const Duration(milliseconds: 240),
          curve: Curves.easeOutCubic,
        );
      }
    }

    void runAfterLayout() {
      scrollToEndOnce();
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!mounted) return;
        scrollToEndOnce();
      });
    }

    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted) return;
      if (!_scrollController.hasClients) {
        WidgetsBinding.instance.addPostFrameCallback((_) {
          if (!mounted) return;
          runAfterLayout();
        });
        return;
      }
      runAfterLayout();
    });
  }

  /// Recording-in-progress bar: cancel · live waveform · timer · stop.
  Widget _voiceRecordingBar(BuildContext context, GhalBolChatRoomPalette p) {
    final elapsedMs = _recordingElapsed.inMilliseconds
        .clamp(0, _maxVoiceDurationMs)
        .toInt();
    return Row(
      children: [
        IconButton(
          onPressed: () => unawaited(_cancelVoiceRecording()),
          tooltip: "Discard",
          icon: const Icon(Icons.delete_outline, color: Colors.redAccent),
          iconSize: 26,
        ),
        Expanded(
          child: Container(
            height: 52,
            padding: const EdgeInsets.fromLTRB(14, 8, 14, 8),
            decoration: BoxDecoration(
              color: p.composerFieldFill,
              borderRadius: BorderRadius.circular(26),
              border: Border.all(color: p.composerBorder),
            ),
            child: Row(
              children: [
                Container(
                  width: 8,
                  height: 8,
                  decoration: const BoxDecoration(
                    color: Colors.redAccent,
                    shape: BoxShape.circle,
                  ),
                ),
                const SizedBox(width: 10),
                Text(
                  _formatDurationMs(elapsedMs),
                  style: TextStyle(
                    color: p.receivedForeground,
                    fontWeight: FontWeight.w700,
                    fontFeatures: const [FontFeature.tabularFigures()],
                  ),
                ),
                const SizedBox(width: 12),
                Expanded(
                  child: _VoiceWaveform(
                    levels: List<double>.from(_liveVoiceLevels),
                    progress: 1.0,
                    activeColor: Colors.redAccent,
                    inactiveColor: Colors.redAccent.withValues(alpha: 0.25),
                    height: 30,
                  ),
                ),
              ],
            ),
          ),
        ),
        const SizedBox(width: 4),
        Material(
          color: p.sendFab,
          shape: const CircleBorder(),
          clipBehavior: Clip.antiAlias,
          elevation: 2,
          shadowColor: Colors.black26,
          child: InkWell(
            onTap: () => unawaited(_stopVoiceRecordingToPreview()),
            customBorder: const CircleBorder(),
            child: const SizedBox(
              width: 48,
              height: 48,
              child: Center(
                child: Icon(Icons.stop_rounded, color: Colors.white, size: 26),
              ),
            ),
          ),
        ),
      ],
    );
  }

  /// Preview bar after recording: trash · play · waveform (seekable) · send.
  Widget _voicePreviewBar(BuildContext context, GhalBolChatRoomPalette p) {
    final total = _voicePreviewDurationMs;
    final posMs = _voicePreviewPosition.inMilliseconds.clamp(0, total).toInt();
    final progress = total > 0 ? posMs / total : 0.0;
    final label = _voicePreviewPlaying || posMs > 0
        ? _formatDurationMs(posMs)
        : _formatDurationMs(total);
    final levels = _voicePreviewLevels.isNotEmpty
        ? _voicePreviewLevels
        : _VoiceWaveform.synthLevels(
            _VoiceWaveform.seedFrom(_voicePreviewPath, null, total),
            count: 40,
          );
    return Row(
      children: [
        IconButton(
          onPressed: () => unawaited(_discardVoicePreview()),
          tooltip: "Discard",
          icon: const Icon(Icons.delete_outline, color: Colors.redAccent),
          iconSize: 26,
        ),
        Expanded(
          child: Container(
            height: 52,
            padding: const EdgeInsets.only(left: 4, right: 14),
            decoration: BoxDecoration(
              color: p.composerFieldFill,
              borderRadius: BorderRadius.circular(26),
              border: Border.all(color: p.composerBorder),
            ),
            child: Row(
              children: [
                IconButton(
                  onPressed: () => unawaited(_toggleVoicePreviewPlayback()),
                  tooltip: _voicePreviewPlaying ? "Pause" : "Play",
                  icon: Icon(
                    _voicePreviewPlaying
                        ? Icons.pause_circle_filled
                        : Icons.play_circle_fill,
                    color: p.sendFab,
                  ),
                  iconSize: 34,
                  padding: EdgeInsets.zero,
                  constraints: const BoxConstraints(minWidth: 40, minHeight: 40),
                ),
                const SizedBox(width: 4),
                Expanded(
                  child: _VoiceWaveform(
                    levels: levels,
                    progress: progress,
                    activeColor: p.sendFab,
                    inactiveColor: p.metaText.withValues(alpha: 0.35),
                    height: 30,
                    onSeek: (f) => unawaited(_seekVoicePreview(f)),
                  ),
                ),
                const SizedBox(width: 10),
                Text(
                  label,
                  style: TextStyle(
                    color: p.receivedForeground,
                    fontWeight: FontWeight.w700,
                    fontFeatures: const [FontFeature.tabularFigures()],
                  ),
                ),
              ],
            ),
          ),
        ),
        const SizedBox(width: 4),
        Material(
          color: p.sendFab,
          shape: const CircleBorder(),
          clipBehavior: Clip.antiAlias,
          elevation: 2,
          shadowColor: Colors.black26,
          child: InkWell(
            onTap: _voiceSending ? null : () => unawaited(_sendVoicePreview()),
            customBorder: const CircleBorder(),
            child: SizedBox(
              width: 48,
              height: 48,
              child: Center(
                child: _voiceSending
                    ? const SizedBox(
                        width: 20,
                        height: 20,
                        child: CircularProgressIndicator(
                          strokeWidth: 2,
                          valueColor: AlwaysStoppedAnimation<Color>(
                            Colors.white,
                          ),
                        ),
                      )
                    : const Icon(
                        Icons.send_rounded,
                        color: Colors.white,
                        size: 24,
                      ),
              ),
            ),
          ),
        ),
      ],
    );
  }

  static String _formatDurationMs(int? durationMs) {
    final totalSeconds = ((durationMs ?? 0) / 1000)
        .round()
        .clamp(0, 5999)
        .toInt();
    final minutes = totalSeconds ~/ 60;
    final seconds = totalSeconds % 60;
    return "$minutes:${seconds.toString().padLeft(2, "0")}";
  }

  Widget _buildChatComposer(
    BuildContext context,
    GhalBolChatRoomPalette p,
    double bottomSafe,
  ) {
    final blocked = _trustContact?.isBlocked == true;
    return Material(
      color: p.composerBar,
      elevation: 12,
      shadowColor: Colors.black38,
      surfaceTintColor: Colors.transparent,
      child: SafeArea(
        top: false,
        minimum: EdgeInsets.zero,
        child: Padding(
          padding: EdgeInsets.fromLTRB(4, 10, 6, 10 + bottomSafe),
          child: _voicePreviewPath != null
              ? _voicePreviewBar(context, p)
              : _isRecordingVoice
              ? _voiceRecordingBar(context, p)
              : Row(
                crossAxisAlignment: CrossAxisAlignment.center,
                children: [
                  IconButton(
                onPressed: blocked ? null : () => unawaited(_pickAndSendAttachment()),
                    tooltip: "Attach",
                    icon: Icon(Icons.add_circle_outline, color: p.metaText),
                    iconSize: 28,
                    constraints: const BoxConstraints(
                      minWidth: 48,
                      minHeight: 48,
                    ),
                    padding: EdgeInsets.zero,
                    style: IconButton.styleFrom(
                      tapTargetSize: MaterialTapTargetSize.padded,
                      visualDensity: VisualDensity.standard,
                    ),
                  ),
                  IconButton(
                    onPressed: () {},
                    tooltip: "Emoji",
                    icon: Icon(Icons.mood_outlined, color: p.metaText),
                    iconSize: 28,
                    constraints: const BoxConstraints(
                      minWidth: 48,
                      minHeight: 48,
                    ),
                    padding: EdgeInsets.zero,
                    style: IconButton.styleFrom(
                      tapTargetSize: MaterialTapTargetSize.padded,
                      visualDensity: VisualDensity.standard,
                    ),
                  ),
                  Expanded(
                    child: Shortcuts(
                      shortcuts: const <ShortcutActivator, Intent>{
                        SingleActivator(LogicalKeyboardKey.enter, shift: true):
                            _ComposerSendIntent(),
                        SingleActivator(
                          LogicalKeyboardKey.numpadEnter,
                          shift: true,
                        ): _ComposerSendIntent(),
                      },
                      child: Actions(
                        actions: <Type, Action<Intent>>{
                          _ComposerSendIntent:
                              CallbackAction<_ComposerSendIntent>(
                                onInvoke: (_) {
                                  if (!blocked) _send();
                                  return null;
                                },
                              ),
                        },
                        child: TextField(
                          controller: _msgCtrl,
                          enabled: !blocked,
                          keyboardType: TextInputType.multiline,
                          textInputAction: TextInputAction.newline,
                          minLines: 1,
                          maxLines: 6,
                          style: TextStyle(
                            color: p.receivedForeground,
                            fontSize: 15,
                          ),
                          cursorColor: p.sendFab,
                          decoration: InputDecoration(
                            filled: true,
                            fillColor: p.composerFieldFill,
                            hintText: blocked ? "Blocked" : "Message",
                            hintStyle: TextStyle(
                              color: p.metaText,
                              fontSize: 15,
                            ),
                            isDense: false,
                            border: OutlineInputBorder(
                              borderRadius: BorderRadius.circular(22),
                              borderSide: BorderSide(color: p.composerBorder),
                            ),
                            enabledBorder: OutlineInputBorder(
                              borderRadius: BorderRadius.circular(22),
                              borderSide: BorderSide(color: p.composerBorder),
                            ),
                            focusedBorder: OutlineInputBorder(
                              borderRadius: BorderRadius.circular(22),
                              borderSide: BorderSide(
                                color: p.sendFab,
                                width: 1.4,
                              ),
                            ),
                            contentPadding: const EdgeInsets.symmetric(
                              horizontal: 16,
                              vertical: 12,
                            ),
                          ),
                        ),
                      ),
                    ),
                  ),
                  const SizedBox(width: 4),
                  Material(
                    color: p.sendFab,
                    shape: const CircleBorder(),
                    clipBehavior: Clip.antiAlias,
                    elevation: 2,
                    shadowColor: Colors.black26,
                    child: InkWell(
                      onTap: blocked
                          ? null
                          : () => unawaited(_startVoiceRecording()),
                      customBorder: const CircleBorder(),
                      child: const SizedBox(
                        width: 48,
                        height: 48,
                        child: Center(
                          child: Icon(
                            Icons.mic,
                            color: Colors.white,
                            size: 24,
                          ),
                        ),
                      ),
                    ),
                  ),
                  const SizedBox(width: 4),
                  Material(
                    color: p.sendFab,
                    shape: const CircleBorder(),
                    clipBehavior: Clip.antiAlias,
                    elevation: 2,
                    shadowColor: Colors.black26,
                    child: InkWell(
                      onTap: blocked ? null : _send,
                      customBorder: const CircleBorder(),
                      child: const SizedBox(
                        width: 48,
                        height: 48,
                        child: Center(
                          child: Icon(
                            Icons.send_rounded,
                            color: Colors.white,
                            size: 24,
                          ),
                        ),
                      ),
                    ),
                  ),
                ],
              ),
        ),
      ),
    );
  }

  Widget _chatRoomPopupMenu(BuildContext context, GhalBolChatRoomPalette p) {
    final callPk = _callPeerPkHex;
    final showCall = GhalBolP2p.isAvailable && callPk != null;
    return PopupMenuButton<String>(
      icon: Icon(Icons.more_vert, color: p.appBarFg),
      tooltip: "Chat options",
      onSelected: (value) {
        switch (value) {
          case "call":
            if (callPk != null) {
              unawaited(
                CallController.instance.startOutgoing(
                  context: context,
                  peerPublicKeyHex: callPk,
                  displayName: _callPeerDisplayName(),
                ),
              );
            }
          case "share":
            if (GhalBolP2p.isAvailable) unawaited(_openShareSheet());
          case "scan":
            if (GhalBolP2p.isAvailable) unawaited(_openJoinFlow());
          case "lock":
            widget.onLock?.call();
        }
      },
      itemBuilder: (ctx) => [
        if (showCall)
          const PopupMenuItem(
            value: "call",
            child: ListTile(
              leading: Icon(Icons.call),
              title: Text("Voice call"),
              contentPadding: EdgeInsets.zero,
              visualDensity: VisualDensity.compact,
            ),
          ),
        const PopupMenuItem(
          value: "share",
          child: ListTile(
            leading: Icon(Icons.qr_code),
            title: Text("Share invitation"),
            contentPadding: EdgeInsets.zero,
            visualDensity: VisualDensity.compact,
          ),
        ),
        const PopupMenuItem(
          value: "scan",
          child: ListTile(
            leading: Icon(Icons.qr_code_scanner),
            title: Text("Join someone"),
            contentPadding: EdgeInsets.zero,
            visualDensity: VisualDensity.compact,
          ),
        ),
        if (widget.onLock != null)
          const PopupMenuItem(
            value: "lock",
            child: ListTile(
              leading: Icon(Icons.lock_outline),
              title: Text("Lock"),
              contentPadding: EdgeInsets.zero,
              visualDensity: VisualDensity.compact,
            ),
          ),
      ],
    );
  }

  @override
  Widget build(BuildContext context) {
    final bottomSafe = MediaQuery.paddingOf(context).bottom;
    final p = GhalBolChatRoomPalette.of(context);
    final mq = MediaQuery.sizeOf(context);

    final PreferredSizeWidget? roomAppBar = widget.networkActionsInHub
        ? null
        : AppBar(
            elevation: 0,
            scrolledUnderElevation: 0,
            backgroundColor: p.appBarBg,
            foregroundColor: p.appBarFg,
            surfaceTintColor: Colors.transparent,
            bottom: PreferredSize(
              preferredSize: const Size.fromHeight(1),
              child: Container(
                height: 1,
                color: p.appBarDivider.withValues(alpha: 0.45),
              ),
            ),
            leading: widget.onLeaveRoom != null
                ? IconButton(
                    tooltip: "Back to chats",
                    icon: Icon(Icons.arrow_back, color: p.appBarFg),
                    onPressed: widget.onLeaveRoom,
                  )
                : null,
            automaticallyImplyLeading: false,
            title: SelectableText(
              ghalBolIdName(
                publicKeyHex: widget.publicKeyHex ?? widget.libp2pPeerId,
                customAlias: _effectiveCustomAlias,
              ),
              style: Theme.of(context).textTheme.titleMedium?.copyWith(
                color: p.appBarFg,
                fontWeight: FontWeight.w600,
              ),
              maxLines: 2,
            ),
            actions: [_chatRoomPopupMenu(context, p)],
          );

    if (widget.networkActionsInHub && widget.activeContact == null) {
      return Scaffold(
        backgroundColor: p.chatBackground,
        body: Center(
          child: Padding(
            padding: const EdgeInsets.all(24),
            child: Text(
              "Select a contact from the list, or tap + to scan an invitation.",
              textAlign: TextAlign.center,
              style: Theme.of(
                context,
              ).textTheme.bodyLarge?.copyWith(color: p.metaText),
            ),
          ),
        ),
      );
    }

    return Scaffold(
      resizeToAvoidBottomInset: true,
      backgroundColor: p.chatBackground,
      appBar: roomAppBar,
      body: Column(
        children: [
          if (widget.networkActionsInHub &&
              widget.activeContact != null &&
              widget.onLeaveRoom == null)
            Material(
              color: p.appBarBg,
              child: Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  Padding(
                    padding: const EdgeInsets.fromLTRB(6, 4, 4, 4),
                    child: Row(
                      children: [
                        Expanded(
                          child: Padding(
                            padding: const EdgeInsets.only(left: 8),
                            child: Text(
                              ghalBolIdName(
                                publicKeyHex:
                                    widget.activeContact!.publicKeyHex,
                                customAlias: widget.activeContact!.displayAlias,
                              ),
                              maxLines: 1,
                              overflow: TextOverflow.ellipsis,
                              style: Theme.of(context).textTheme.titleSmall
                                  ?.copyWith(
                                    color: p.appBarFg,
                                    fontWeight: FontWeight.w600,
                                  ),
                            ),
                          ),
                        ),
                        _chatRoomPopupMenu(context, p),
                      ],
                    ),
                  ),
                  Container(
                    height: 1,
                    color: p.appBarDivider.withValues(alpha: 0.45),
                  ),
                ],
              ),
            ),
          _connectionStatusBar(context),
          _contactTrustBanner(context, p),
          Expanded(
            child: Stack(
              fit: StackFit.expand,
              children: [
                CustomPaint(
                  painter: ChatWallpaperPainter(
                    background: p.chatBackground,
                    isDark: p.isDark,
                  ),
                ),
                if (_loadingTranscript &&
                    _lines.where((l) => !l.system).isEmpty)
                  Center(
                    child: Column(
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        const CircularProgressIndicator(),
                        const SizedBox(height: 12),
                        Text(
                          "Loading conversation…",
                          style: Theme.of(
                            context,
                          ).textTheme.bodyMedium?.copyWith(color: p.metaText),
                        ),
                      ],
                    ),
                  ),
                SelectionArea(
                  child: ListView.builder(
                    controller: _scrollController,
                    reverse: true,
                    padding: const EdgeInsets.symmetric(
                      horizontal: 6,
                      vertical: 8,
                    ),
                    itemCount: _lines.length,
                    itemBuilder: (ctx, i) {
                      final l = _lines[_lines.length - 1 - i];
                      if (l.system) {
                        return Center(
                          child: Padding(
                            padding: const EdgeInsets.symmetric(
                              vertical: 6,
                              horizontal: 20,
                            ),
                            child: DecoratedBox(
                              decoration: BoxDecoration(
                                color: p.systemChipBg,
                                borderRadius: BorderRadius.circular(8),
                                boxShadow: [
                                  BoxShadow(
                                    color: Colors.black.withValues(
                                      alpha: p.isDark ? 0.35 : 0.06,
                                    ),
                                    blurRadius: 2,
                                    offset: const Offset(0, 1),
                                  ),
                                ],
                              ),
                              child: Padding(
                                padding: const EdgeInsets.symmetric(
                                  horizontal: 10,
                                  vertical: 6,
                                ),
                                child: Text(
                                  l.text,
                                  textAlign: TextAlign.center,
                                  style: TextStyle(
                                    color: p.systemChipFg,
                                    fontSize: 12.5,
                                    height: 1.35,
                                  ),
                                  maxLines: 32,
                                  overflow: TextOverflow.ellipsis,
                                ),
                              ),
                            ),
                          ),
                        );
                      }
                      final outgoing = _isOutgoingMessage(l);
                      final bubble = outgoing ? p.sentBubble : p.receivedBubble;
                      final fg = outgoing
                          ? p.sentForeground
                          : p.receivedForeground;
                      final mid = l.messageId?.trim() ?? "";
                      final rowKey = mid.isNotEmpty
                          ? "m:$mid"
                          : "l:${l.localId}";
                      return KeyedSubtree(
                        key: ValueKey<String>(rowKey),
                        child: Padding(
                          padding: EdgeInsets.fromLTRB(
                            outgoing ? 52 : 8,
                            3,
                            outgoing ? 8 : 52,
                            3,
                          ),
                          child: Align(
                            alignment: outgoing
                                ? Alignment.centerRight
                                : Alignment.centerLeft,
                            child: DecoratedBox(
                              decoration: BoxDecoration(
                                color: bubble,
                                borderRadius: BorderRadius.only(
                                  topLeft: const Radius.circular(10),
                                  topRight: const Radius.circular(10),
                                  bottomLeft: Radius.circular(
                                    outgoing ? 10 : 3,
                                  ),
                                  bottomRight: Radius.circular(
                                    outgoing ? 3 : 10,
                                  ),
                                ),
                                boxShadow: [
                                  BoxShadow(
                                    color: Colors.black.withValues(
                                      alpha: p.isDark ? 0.28 : 0.07,
                                    ),
                                    blurRadius: 2,
                                    offset: const Offset(0, 1),
                                  ),
                                ],
                              ),
                              child: ConstrainedBox(
                                constraints: BoxConstraints(
                                  // Voice notes read better shorter; text may use more.
                                  maxWidth: mq.width * (l.isVoice ? 0.70 : 0.82),
                                ),
                                child: Padding(
                                  padding: const EdgeInsets.fromLTRB(
                                    10,
                                    7,
                                    10,
                                    6,
                                  ),
                                  child: Column(
                                    crossAxisAlignment: outgoing
                                        ? CrossAxisAlignment.end
                                        : CrossAxisAlignment.start,
                                    children: [
                                      if (l.isVoice)
                                        _VoiceBubble(
                                          line: l,
                                          foreground: fg,
                                          metaColor: p.metaText,
                                        )
                                      else if (l.isAttachment)
                                        _attachmentBubble(l, fg, p.metaText)
                                      else
                                        Text(
                                          l.text,
                                          style: TextStyle(
                                            color: fg,
                                            height: 1.38,
                                            fontSize: 15,
                                          ),
                                          maxLines: 48,
                                          overflow: TextOverflow.ellipsis,
                                        ),
                                      const SizedBox(height: 3),
                                      Row(
                                        mainAxisSize: MainAxisSize.min,
                                        children: [
                                          Flexible(
                                            child: Text(
                                              _bubbleSenderCaption(l),
                                              style: TextStyle(
                                                color: p.metaText,
                                                fontSize: 11,
                                              ),
                                              maxLines: 1,
                                              overflow: TextOverflow.ellipsis,
                                            ),
                                          ),
                                          if (outgoing) ...[
                                            const SizedBox(width: 4),
                                            _deliveryTicks(
                                              l,
                                              p.metaText,
                                              failedColor: p.sendFab,
                                            ),
                                          ],
                                        ],
                                      ),
                                    ],
                                  ),
                                ),
                              ),
                            ),
                          ),
                        ),
                      );
                    },
                  ),
                ),
              ],
            ),
          ),
          _buildChatComposer(context, p, bottomSafe),
        ],
      ),
    );
  }

  /// Invoked from [ChatHubScreen] when network actions live on the shell app bar.
  Future<void> requestJoinFlow() => _openJoinFlow();

  Future<void> requestShareInvitation() => _openShareSheet();

  Future<void> requestCopyInvitationLink() => copyInvitationLink();

  Future<void> requestShareInvitationLink() => shareInvitationLink();
}

/// Full-screen local image viewer (Android cannot open app-private `file://` URIs).
class _AttachmentImageViewer extends StatelessWidget {
  const _AttachmentImageViewer({
    required this.path,
    required this.title,
    this.onSave,
  });

  final String path;
  final String title;
  final Future<void> Function()? onSave;

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      backgroundColor: Colors.black,
      appBar: AppBar(
        backgroundColor: Colors.black,
        foregroundColor: Colors.white,
        title: Text(
          title.isEmpty ? "Attachment" : title,
          maxLines: 1,
          overflow: TextOverflow.ellipsis,
        ),
        actions: [
          if (onSave != null)
            IconButton(
              tooltip: "Save",
              icon: const Icon(Icons.save_alt_rounded),
              onPressed: () => unawaited(onSave!()),
            ),
        ],
      ),
      body: Center(
        child: InteractiveViewer(
          minScale: 0.5,
          maxScale: 5,
          child: Image.file(
            File(path),
            fit: BoxFit.contain,
            errorBuilder: (_, _, _) => const Padding(
              padding: EdgeInsets.all(24),
              child: Text(
                "Could not load image",
                style: TextStyle(color: Colors.white70),
              ),
            ),
          ),
        ),
      ),
    );
  }
}
