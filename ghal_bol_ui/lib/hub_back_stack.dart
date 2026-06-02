/// Browser-style back stack inside [ChatHubScreen], separate from [Navigator] routes.
///
/// System back (Android edge gesture, desktop mouse back) should pop pushed routes
/// first, then unwind this stack, then exit the app.
enum HubSyntheticBackResult {
  /// Narrow: chat room → list. Wide: disengage chat column (foreground peer cleared).
  leaveChatRoom,

  /// Identity / More → Chats tab.
  popToChatsTab,

  /// At hub root (chats list); allow system to exit the app.
  none,
}

/// Whether the hub shell still has an internal level to unwind.
bool hubHasSyntheticBack({
  required bool shellSplit,
  required int navTab,
  required bool narrowShowRoom,
  required bool splitChatEngaged,
  required bool hasSelectedContact,
}) {
  return hubSyntheticBackResult(
        shellSplit: shellSplit,
        navTab: navTab,
        narrowShowRoom: narrowShowRoom,
        splitChatEngaged: splitChatEngaged,
        hasSelectedContact: hasSelectedContact,
      ) !=
      HubSyntheticBackResult.none;
}

/// Next synthetic back step, or [HubSyntheticBackResult.none] at root.
HubSyntheticBackResult hubSyntheticBackResult({
  required bool shellSplit,
  required int navTab,
  required bool narrowShowRoom,
  required bool splitChatEngaged,
  required bool hasSelectedContact,
}) {
  if (shellSplit) {
    if (navTab == 0 && splitChatEngaged && hasSelectedContact) {
      return HubSyntheticBackResult.leaveChatRoom;
    }
  } else {
    if (navTab == 0 && narrowShowRoom) {
      return HubSyntheticBackResult.leaveChatRoom;
    }
  }
  if (navTab != 0) {
    return HubSyntheticBackResult.popToChatsTab;
  }
  return HubSyntheticBackResult.none;
}

/// [Navigator] may pop a route, or the hub may exit — not blocked by synthetic stack.
bool hubAllowsSystemPop({
  required bool navigatorCanPop,
  required bool shellSplit,
  required int navTab,
  required bool narrowShowRoom,
  required bool splitChatEngaged,
  required bool hasSelectedContact,
}) {
  if (navigatorCanPop) return true;
  return !hubHasSyntheticBack(
    shellSplit: shellSplit,
    navTab: navTab,
    narrowShowRoom: narrowShowRoom,
    splitChatEngaged: splitChatEngaged,
    hasSelectedContact: hasSelectedContact,
  );
}
