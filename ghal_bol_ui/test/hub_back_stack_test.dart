import "package:flutter_test/flutter_test.dart";
import "package:ghal_bol_ui/hub_back_stack.dart";

void main() {
  group("hubHasSyntheticBack", () {
    test("narrow chat room", () {
      expect(
        hubHasSyntheticBack(
          shellSplit: false,
          navTab: 0,
          narrowShowRoom: true,
          splitChatEngaged: false,
          hasSelectedContact: true,
        ),
        isTrue,
      );
    });

    test("narrow chats list root", () {
      expect(
        hubHasSyntheticBack(
          shellSplit: false,
          navTab: 0,
          narrowShowRoom: false,
          splitChatEngaged: false,
          hasSelectedContact: true,
        ),
        isFalse,
      );
    });

    test("wide engaged chat", () {
      expect(
        hubHasSyntheticBack(
          shellSplit: true,
          navTab: 0,
          narrowShowRoom: false,
          splitChatEngaged: true,
          hasSelectedContact: true,
        ),
        isTrue,
      );
    });

    test("wide list without engagement", () {
      expect(
        hubHasSyntheticBack(
          shellSplit: true,
          navTab: 0,
          narrowShowRoom: false,
          splitChatEngaged: false,
          hasSelectedContact: true,
        ),
        isFalse,
      );
    });

    test("non-chats tab", () {
      expect(
        hubHasSyntheticBack(
          shellSplit: false,
          navTab: 2,
          narrowShowRoom: false,
          splitChatEngaged: false,
          hasSelectedContact: false,
        ),
        isTrue,
      );
    });
  });

  group("hubSyntheticBackResult", () {
    test("narrow room then tab", () {
      expect(
        hubSyntheticBackResult(
          shellSplit: false,
          navTab: 0,
          narrowShowRoom: true,
          splitChatEngaged: false,
          hasSelectedContact: true,
        ),
        HubSyntheticBackResult.leaveChatRoom,
      );
      expect(
        hubSyntheticBackResult(
          shellSplit: false,
          navTab: 1,
          narrowShowRoom: false,
          splitChatEngaged: false,
          hasSelectedContact: false,
        ),
        HubSyntheticBackResult.popToChatsTab,
      );
    });
  });

  group("hubAllowsSystemPop", () {
    test("navigator route wins", () {
      expect(
        hubAllowsSystemPop(
          navigatorCanPop: true,
          shellSplit: false,
          navTab: 0,
          narrowShowRoom: true,
          splitChatEngaged: false,
          hasSelectedContact: true,
        ),
        isTrue,
      );
    });

    test("blocked on synthetic stack", () {
      expect(
        hubAllowsSystemPop(
          navigatorCanPop: false,
          shellSplit: false,
          navTab: 0,
          narrowShowRoom: true,
          splitChatEngaged: false,
          hasSelectedContact: true,
        ),
        isFalse,
      );
    });
  });
}
