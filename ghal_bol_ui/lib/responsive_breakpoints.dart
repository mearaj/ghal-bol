import "package:flutter/widgets.dart";

/// Width at or above which the UI uses a **two-column** shell (chat list + room), like a
/// resized desktop window or a large phone / tablet in landscape.
///
/// Below this width, the app uses a **stacked** flow: list first, then full-screen room with back.
const double kGhalBolChatShellSplitWidth = 720;

bool ghalBolUseChatShellSplit(BuildContext context) {
  return MediaQuery.sizeOf(context).width >= kGhalBolChatShellSplitWidth;
}
