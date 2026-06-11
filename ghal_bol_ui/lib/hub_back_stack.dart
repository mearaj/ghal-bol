/// Browser-style history inside [ChatHubScreen] (separate from [Navigator] routes).
///
/// Each forward navigation (tab, open chat room, wide-pane engage) appends a snapshot.
/// System back / toolbar back pops one snapshot at a time, like the browser Back button.
///
/// **Chat / acks:** This stack only stores hub chrome flags. Every pop/apply must still
/// call [ChatHubScreenState]'s native session sync via [GhalBolUiSession] — history does not replace that path.
class HubHistoryEntry {
  const HubHistoryEntry({
    required this.navTab,
    required this.narrowShowRoom,
    required this.splitChatEngaged,
    this.conversationKey,
  });

  final int navTab;
  final bool narrowShowRoom;
  final bool splitChatEngaged;

  /// Roster thread key when a row is selected (`public_key_hex`).
  final String? conversationKey;

  @override
  bool operator ==(Object other) =>
      other is HubHistoryEntry &&
      navTab == other.navTab &&
      narrowShowRoom == other.narrowShowRoom &&
      splitChatEngaged == other.splitChatEngaged &&
      conversationKey == other.conversationKey;

  @override
  int get hashCode => Object.hash(navTab, narrowShowRoom, splitChatEngaged, conversationKey);
}

/// In-memory back/forward stack for hub chrome (max depth avoids runaway growth).
class HubHistoryStack {
  HubHistoryStack({this.maxDepth = 64});

  final int maxDepth;
  final List<HubHistoryEntry> _entries = [];

  HubHistoryEntry? get current => _entries.isEmpty ? null : _entries.last;

  bool get canGoBack => _entries.length > 1;

  /// Hub at root (only the initial entry).
  bool get isAtRoot => _entries.length <= 1;

  void reset(HubHistoryEntry entry) {
    _entries
      ..clear()
      ..add(entry);
  }

  /// Replace the top entry without adding history (layout / sync only).
  ///
  /// If the new top matches the entry below, drops the duplicate (same as browser
  /// `replaceState` collapsing identical URLs).
  void replaceTop(HubHistoryEntry entry) {
    if (_entries.isEmpty) {
      _entries.add(entry);
      return;
    }
    _entries[_entries.length - 1] = entry;
    if (_entries.length >= 2 && _entries[_entries.length - 2] == entry) {
      _entries.removeLast();
    }
  }

  /// Record a forward navigation; skips duplicate consecutive entries.
  void recordNavigate(HubHistoryEntry entry) {
    if (_entries.isEmpty) {
      _entries.add(entry);
      return;
    }
    if (_entries.last == entry) return;
    _entries.add(entry);
    while (_entries.length > maxDepth) {
      _entries.removeAt(1);
    }
  }

  /// Pop one level; returns the new top, or `null` if already at root.
  HubHistoryEntry? pop() {
    if (_entries.length <= 1) return null;
    _entries.removeLast();
    return _entries.last;
  }
}
