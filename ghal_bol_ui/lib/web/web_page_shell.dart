import "package:flutter/material.dart";

/// Centered column for responsive web pages (phone → desktop).
class WebPageShell extends StatelessWidget {
  const WebPageShell({
    super.key,
    required this.child,
    /// Wide enough for the 1536×1024 hero on desktop; still capped on narrow phones.
    this.maxWidth = 1152,
  });

  final Widget child;
  final double maxWidth;

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      body: SafeArea(
        child: LayoutBuilder(
          builder: (context, constraints) {
            final pad = constraints.maxWidth < 400 ? 20.0 : 32.0;
            return SingleChildScrollView(
              padding: EdgeInsets.symmetric(horizontal: pad, vertical: 28),
              child: Center(
                child: ConstrainedBox(
                  constraints: BoxConstraints(maxWidth: maxWidth),
                  child: child,
                ),
              ),
            );
          },
        ),
      ),
    );
  }
}
