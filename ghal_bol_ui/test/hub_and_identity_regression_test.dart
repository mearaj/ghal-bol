import "package:flutter_test/flutter_test.dart";
import "package:ghal_bol_ui/hub_roster_selection.dart";
import "package:ghal_bol_ui/public_key_hex.dart";

void main() {
  const pkA =
      "0324653eac434488002cc06bbfb7f10fe18991e35f9fe4302dbea6d2353dc0ab1c";
  const pkB =
      "0224653eac434488002cc06bbfb7f10fe18991e35f9fe4302dbea6d2353dc0ab1d";

  group("preserveHubConversationSelection", () {
    test("keeps scanned contact when roster reload races before row appears", () {
      expect(
        preserveHubConversationSelection(
          selectedConversationKey: pkA,
          rosterKeys: [pkB],
        ),
        pkA,
      );
    });

    test("does not substitute list.first when selection missing from roster", () {
      final kept = preserveHubConversationSelection(
        selectedConversationKey: pkA,
        rosterKeys: [pkB],
      );
      expect(kept, isNot(pkB));
      expect(kept, pkA);
    });
  });

  group("contactPublicKeyHexFromEvent", () {
    test("uses sender_public_key_hex not libp2p from field", () {
      final pk = contactPublicKeyHexFromEvent({
        "kind": "dm_message",
        "sender_public_key_hex": pkA,
        "from": "12D3KooWWrongContactMustNotUseThisAsIdentity",
      });
      expect(pk, pkA);
    });

    test("ignores peer_id when sender key absent", () {
      expect(
        contactPublicKeyHexFromEvent({
          "kind": "peer_connected",
          "peer_id": "12D3KooWExample",
        }),
        isEmpty,
      );
    });

    test("uses public_key_hex on connect events", () {
      expect(
        contactPublicKeyHexFromEvent({
          "kind": "peer_connected",
          "public_key_hex": pkB,
          "peer_id": "12D3KooWExample",
        }),
        pkB,
      );
    });
  });
}
