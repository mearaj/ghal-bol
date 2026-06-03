import "package:flutter/material.dart";

/// Play feature graphic (1536×1024) at the top of web home and invite pages.
///
/// Decodes at the on-screen pixel size (DPR-aware) so downscaling stays sharp.
class WebHeaderGraphic extends StatelessWidget {
  const WebHeaderGraphic({super.key});

  static const assetPath = "assets/for-feature-graphic-1.png";

  static const intrinsicW = 1536;
  static const intrinsicH = 1024;

  @override
  Widget build(BuildContext context) {
    final colorScheme = Theme.of(context).colorScheme;
    final dpr = MediaQuery.devicePixelRatioOf(context);

    return LayoutBuilder(
      builder: (context, constraints) {
        final layoutW = constraints.maxWidth.isFinite && constraints.maxWidth > 0
            ? constraints.maxWidth
            : MediaQuery.sizeOf(context).width;
        final cacheW = (layoutW * dpr).round().clamp(1, intrinsicW);
        final cacheH = (cacheW * intrinsicH / intrinsicW).round().clamp(1, intrinsicH);

        return ClipRRect(
          borderRadius: BorderRadius.circular(12),
          child: AspectRatio(
            aspectRatio: intrinsicW / intrinsicH,
            child: Image.asset(
              assetPath,
              fit: BoxFit.fitWidth,
              alignment: Alignment.center,
              filterQuality: FilterQuality.high,
              cacheWidth: cacheW,
              cacheHeight: cacheH,
              errorBuilder: (_, _, _) => ColoredBox(
                color: colorScheme.surfaceContainerHighest,
                child: Icon(
                  Icons.image_not_supported,
                  color: colorScheme.primary,
                  size: 48,
                ),
              ),
            ),
          ),
        );
      },
    );
  }
}
