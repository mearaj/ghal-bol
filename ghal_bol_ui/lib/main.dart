import "bootstrap_native.dart" if (dart.library.html) "bootstrap_web.dart" as bootstrap;

/// Android, iOS, Linux, desktop — or static web site when compiled for HTML.
void main() {
  bootstrap.runGhalBol();
}
