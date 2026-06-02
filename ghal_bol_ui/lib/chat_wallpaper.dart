import "package:flutter/material.dart";

/// WhatsApp-inspired chat background (light / dark).
class ChatWallpaperPainter extends CustomPainter {
  ChatWallpaperPainter({required this.background, required this.isDark});

  final Color background;
  final bool isDark;

  @override
  void paint(Canvas canvas, Size size) {
    canvas.drawRect(Offset.zero & size, Paint()..color = background);

    final ink = isDark ? const Color(0x14FFFFFF) : const Color(0x12000000);
    final stroke = Paint()
      ..color = ink
      ..strokeWidth = 1.2
      ..style = PaintingStyle.stroke;

    const step = 52.0;
    for (var y = 0.0; y < size.height + step; y += step) {
      for (var x = 0.0; x < size.width + step; x += step) {
        final cx = x + (y * 0.17 % 18);
        final cy = y + (x * 0.11 % 14);
        canvas.drawCircle(Offset(cx + 8, cy + 6), 2.2, Paint()..color = ink);
        final r = RRect.fromRectAndRadius(
          Rect.fromCenter(center: Offset(cx + 28, cy + 22), width: 18, height: 10),
          const Radius.circular(3),
        );
        canvas.drawRRect(r, stroke);
        canvas.drawArc(
          Rect.fromCenter(center: Offset(cx + 22, cy + 38), width: 22, height: 22),
          0.4,
          1.9,
          false,
          stroke,
        );
      }
    }
  }

  @override
  bool shouldRepaint(covariant ChatWallpaperPainter oldDelegate) =>
      oldDelegate.isDark != isDark || oldDelegate.background != background;
}

/// Bubble + wallpaper colors aligned with common chat-app light/dark themes.
class GhalBolChatRoomPalette {
  GhalBolChatRoomPalette._(this.isDark);

  factory GhalBolChatRoomPalette.of(BuildContext context) {
    return GhalBolChatRoomPalette._(Theme.of(context).brightness == Brightness.dark);
  }

  final bool isDark;

  Color get chatBackground => isDark ? const Color(0xFF0B141A) : const Color(0xFFECE5DD);

  Color get sentBubble => isDark ? const Color(0xFF005C4B) : const Color(0xFFDCF8C6);

  Color get receivedBubble => isDark ? const Color(0xFF202C33) : const Color(0xFFFFFFFF);

  Color get sentForeground => isDark ? const Color(0xFFE9EDEF) : const Color(0xFF111B21);

  Color get receivedForeground => isDark ? const Color(0xFFE9EDEF) : const Color(0xFF111B21);

  Color get metaText => isDark ? const Color(0xFF8696A0) : const Color(0xFF667781);

  Color get systemChipBg => isDark ? const Color(0xFF182229) : const Color(0xFFEEE8DD);

  Color get systemChipFg => isDark ? const Color(0xFFB7BFC4) : const Color(0xFF54656F);

  Color get composerBar => isDark ? const Color(0xFF1F2C34) : const Color(0xFFF0F0F0);

  Color get composerFieldFill => isDark ? const Color(0xFF2A3942) : Colors.white;

  Color get composerBorder => isDark ? const Color(0xFF2A3942) : const Color(0xFFE9E9E9);

  Color get sendFab => isDark ? const Color(0xFF00A884) : const Color(0xFF008069);

  Color get appBarBg => isDark ? const Color(0xFF1F2C34) : const Color(0xFFF7F7F7);

  Color get appBarFg => isDark ? const Color(0xFFE9EDEF) : const Color(0xFF111B21);

  /// Subtle divider under app bar.
  Color get appBarDivider => Color.lerp(appBarBg, Colors.black, isDark ? 0.12 : 0.06)!;

  /// Chat list pane (pre-room), WhatsApp-style white in light mode.
  Color get hubListBackground => isDark ? const Color(0xFF111B21) : Colors.white;
}
