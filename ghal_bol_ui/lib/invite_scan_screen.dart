import "package:flutter/material.dart";
import "package:ghal_bol_ui/ghal_bol_connect_invite.dart";
import "package:mobile_scanner/mobile_scanner.dart";

/// Full-screen QR reader — avoids tiny dialog previews that often fail on Android.
class InviteScanScreen extends StatefulWidget {
  const InviteScanScreen({super.key});

  /// Pull a connect invite URI from QR text or pasted content.
  static String? extractInviteUri(String? raw) => extractConnectInviteUri(raw);

  @override
  State<InviteScanScreen> createState() => _InviteScanScreenState();
}

class _InviteScanScreenState extends State<InviteScanScreen> {
  bool _accepted = false;

  late final MobileScannerController _controller = MobileScannerController(
    formats: const <BarcodeFormat>[BarcodeFormat.qrCode],
    facing: CameraFacing.back,
    detectionSpeed: DetectionSpeed.normal,
    cameraResolution: const Size(1920, 1080),
  );

  DateTime? _lastAccept;

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  Future<void> _acceptUri(String uri) async {
    final now = DateTime.now();
    if (_lastAccept != null && now.difference(_lastAccept!) < const Duration(milliseconds: 500)) {
      return;
    }
    _lastAccept = now;
    if (!mounted || _accepted) return;
    setState(() => _accepted = true);
    // Brief success state so the camera going black is not mistaken for a crash.
    await Future<void>.delayed(const Duration(milliseconds: 280));
    if (!mounted) return;
    Navigator.of(context).pop<String>(uri);
  }

  void _onDetect(BarcodeCapture capture) {
    for (final b in capture.barcodes) {
      // Prefer raw QR payload — displayValue may drop ?alias= on some devices.
      for (final candidate in [b.rawValue, b.displayValue]) {
        final uri = InviteScanScreen.extractInviteUri(candidate);
        if (uri != null) {
          _acceptUri(uri);
          return;
        }
      }
    }
  }

  Future<void> _pasteInstead() async {
    final ctrl = TextEditingController();
    final got = await showDialog<String>(
      context: context,
      builder: (ctx) => AlertDialog(
        title: const Text("Paste invitation link"),
        content: TextField(
          controller: ctrl,
          autofocus: true,
          decoration: const InputDecoration(
            hintText: "https://ghalbol.com/connect/…",
          ),
          maxLines: 4,
        ),
        actions: [
          TextButton(onPressed: () => Navigator.pop(ctx), child: const Text("Cancel")),
          FilledButton(
            onPressed: () => Navigator.pop(ctx, ctrl.text.trim()),
            child: const Text("Use link"),
          ),
        ],
      ),
    );
    if (!mounted || got == null || got.isEmpty) return;
    final uri = InviteScanScreen.extractInviteUri(got) ?? got.trim();
    if (GhalBolConnectInvite.tryParseInviteUri(uri) != null) {
      _acceptUri(uri);
    } else {
      ScaffoldMessenger.of(context).showSnackBar(
        const SnackBar(
          content: Text(
            "That does not look like a Ghal Bol invite link.",
          ),
        ),
      );
    }
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      backgroundColor: Colors.black,
      appBar: AppBar(
        backgroundColor: Colors.black87,
        foregroundColor: Colors.white,
        title: const Text("Scan invitation"),
        actions: [
          TextButton(
            onPressed: _pasteInstead,
            child: const Text("Paste", style: TextStyle(color: Colors.white)),
          ),
        ],
      ),
      body: Stack(
        fit: StackFit.expand,
        children: [
          if (!_accepted)
            MobileScanner(controller: _controller, onDetect: _onDetect),
          if (_accepted)
            ColoredBox(
              color: Colors.black,
              child: Center(
                child: Column(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    Icon(Icons.check_circle, color: Colors.green.shade400, size: 72),
                    const SizedBox(height: 16),
                    Text(
                      "Invitation read",
                      style: Theme.of(context).textTheme.titleLarge?.copyWith(
                            color: Colors.white,
                            fontWeight: FontWeight.w600,
                          ),
                    ),
                  ],
                ),
              ),
            ),
          if (!_accepted)
            const SafeArea(
              child: Align(
                alignment: Alignment.bottomCenter,
                child: Padding(
                  padding: EdgeInsets.all(24),
                  child: Text(
                    "Point at the other person's QR code",
                    style: TextStyle(color: Colors.white, fontSize: 16),
                    textAlign: TextAlign.center,
                  ),
                ),
              ),
            ),
        ],
      ),
    );
  }
}
