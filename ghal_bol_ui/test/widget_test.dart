import "package:flutter_test/flutter_test.dart";

import "package:ghal_bol_ui/bootstrap_native.dart";

void main() {
  testWidgets("home shows identity title", (WidgetTester tester) async {
    await tester.pumpWidget(const GhalBolApp());
    await tester.pump();
    expect(find.text("Ghal Bol identity"), findsOneWidget);
  });
}
