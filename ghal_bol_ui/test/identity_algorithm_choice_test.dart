import "package:flutter_test/flutter_test.dart";
import "package:ghal_bol_ui/identity_algorithm_choice.dart";

void main() {
  group("IdentityAlgorithms", () {
    test("fallback lists secp256k1 when native unavailable", () {
      final list = IdentityAlgorithms.supported();
      expect(list, isNotEmpty);
      expect(list.first.wireId, IdentityAlgorithms.defaultWireId);
      expect(list.first.p2pReady, isTrue);
    });

    test("defaultOption prefers isDefault flag", () {
      final opt = IdentityAlgorithms.defaultOption();
      expect(opt.wireId, IdentityAlgorithms.defaultWireId);
      expect(opt.isDefault, isTrue);
    });

    test("byWireId returns null for unknown", () {
      expect(IdentityAlgorithms.byWireId("rsa2048"), isNull);
    });
  });
}
