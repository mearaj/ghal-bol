import "package:flutter_test/flutter_test.dart";
import "package:ghal_bol_ui/hub_back_stack.dart";

void main() {
  group("HubHistoryStack", () {
    test("starts at root", () {
      final h = HubHistoryStack();
      h.reset(const HubHistoryEntry(navTab: 0, narrowShowRoom: false, splitChatEngaged: false));
      expect(h.isAtRoot, isTrue);
      expect(h.canGoBack, isFalse);
    });

    test("browser-style tab chain", () {
      final h = HubHistoryStack();
      const list = HubHistoryEntry(navTab: 0, narrowShowRoom: false, splitChatEngaged: false);
      const identity = HubHistoryEntry(navTab: 1, narrowShowRoom: false, splitChatEngaged: false);
      const more = HubHistoryEntry(navTab: 2, narrowShowRoom: false, splitChatEngaged: false);
      h.reset(list);
      h.recordNavigate(identity);
      h.recordNavigate(more);
      expect(h.canGoBack, isTrue);

      expect(h.pop(), identity);
      expect(h.current, identity);

      expect(h.pop(), list);
      expect(h.isAtRoot, isTrue);
      expect(h.pop(), isNull);
    });

    test("narrow room then back to list", () {
      final h = HubHistoryStack();
      const list = HubHistoryEntry(navTab: 0, narrowShowRoom: false, splitChatEngaged: false);
      const room = HubHistoryEntry(navTab: 0, narrowShowRoom: true, splitChatEngaged: false);
      h.reset(list);
      h.recordNavigate(room);
      expect(h.pop(), list);
      expect(h.current?.narrowShowRoom, isFalse);
    });

    test("different chats in room are separate history steps", () {
      final h = HubHistoryStack();
      const roomA = HubHistoryEntry(
        navTab: 0,
        narrowShowRoom: true,
        splitChatEngaged: false,
        conversationKey: "aa",
      );
      const roomB = HubHistoryEntry(
        navTab: 0,
        narrowShowRoom: true,
        splitChatEngaged: false,
        conversationKey: "bb",
      );
      h.reset(roomA);
      h.recordNavigate(roomB);
      expect(h.pop(), roomA);
    });

    test("skips duplicate consecutive entries", () {
      final h = HubHistoryStack();
      const a = HubHistoryEntry(navTab: 0, narrowShowRoom: false, splitChatEngaged: false);
      h.reset(a);
      h.recordNavigate(a);
      expect(h.isAtRoot, isTrue);
    });

    test("pop at root returns null", () {
      final h = HubHistoryStack();
      h.reset(const HubHistoryEntry(navTab: 0, narrowShowRoom: false, splitChatEngaged: false));
      expect(h.pop(), isNull);
    });

    test("replaceTop does not add depth", () {
      final h = HubHistoryStack();
      const list = HubHistoryEntry(navTab: 0, narrowShowRoom: false, splitChatEngaged: false);
      const room = HubHistoryEntry(navTab: 0, narrowShowRoom: true, splitChatEngaged: false);
      h.reset(list);
      h.recordNavigate(room);
      expect(h.canGoBack, isTrue);
      h.replaceTop(list);
      expect(h.isAtRoot, isTrue);
      expect(h.pop(), isNull);
    });
  });
}
