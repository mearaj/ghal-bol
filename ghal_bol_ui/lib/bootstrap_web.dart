import "package:flutter/material.dart";
import "package:flutter_web_plugins/flutter_web_plugins.dart";

import "package:ghal_bol_ui/web/ghal_bol_web_app.dart";

/// Web-only entry (marketing site). Not compiled into Android/desktop builds.
Future<void> runGhalBol() async {
  WidgetsFlutterBinding.ensureInitialized();
  usePathUrlStrategy();
  runApp(const GhalBolWebApp());
}
