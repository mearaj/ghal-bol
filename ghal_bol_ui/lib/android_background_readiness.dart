import "dart:async";
import "dart:io";

import "package:flutter/material.dart";
import "package:ghal_bol_ui/app_log.dart";
import "package:ghal_bol_ui/embedder_storage.dart";
import "package:permission_handler/permission_handler.dart";

enum _ReadinessChoice { later, open }

/// Sequential Android onboarding for permissions/settings needed while the screen is off.
///
/// Skips steps already satisfied. Never shows two prompts at once.
class AndroidBackgroundReadiness {
  AndroidBackgroundReadiness._();

  static bool _flowInFlight = false;

  static Future<void> runIfNeeded(BuildContext context) async {
    if (!Platform.isAndroid) return;
    if (_flowInFlight) return;
    if (!context.mounted) return;

    _flowInFlight = true;
    try {
      await _runNotificationsStep(context);
      if (!context.mounted) return;

      await _runBatteryStep(context);
      if (!context.mounted) return;

      await _runUnusedPauseStep(context);
      if (!context.mounted) return;

      await _runOemBackgroundStep(context);
    } finally {
      _flowInFlight = false;
    }
  }

  static Future<void> _runNotificationsStep(BuildContext context) async {
    if (await Permission.notification.isGranted) return;
    AppLog.instance.flow("Background", "requesting notification permission");
    await Permission.notification.request();
  }

  static Future<void> _runBatteryStep(BuildContext context) async {
    if (!await isBatteryOptimized()) return;
    if (!context.mounted) return;
    AppLog.instance.flow("Background", "battery optimization active — prompting");
    final choice = await _prompt(
      context,
      title: "Allow background messaging",
      body:
          "Android battery optimization can stop Ghal Bol from receiving messages "
          "when the screen is off.\n\n"
          "On the next screen, choose Allow so messaging stays reliable.",
      openLabel: "Allow",
    );
    if (choice != _ReadinessChoice.open || !context.mounted) return;
    await requestBatteryOptimizationExemption();
    await _waitForForegroundReturn();
  }

  static Future<void> _runUnusedPauseStep(BuildContext context) async {
    if (!await isUnusedAppPauseEnabled()) return;
    if (!context.mounted) return;
    AppLog.instance.flow("Background", "unused-app pause enabled — prompting");
    final choice = await _prompt(
      context,
      title: "Disable unused-app pause",
      body:
          '"Pause app activity if unused" is enabled for Ghal Bol. '
          "That can block the messaging service when the app has not been opened recently.\n\n"
          "Turn it off on the next screen.",
      openLabel: "Open settings",
    );
    if (choice != _ReadinessChoice.open || !context.mounted) return;
    await openUnusedAppSettings();
    await _waitForForegroundReturn();
  }

  static Future<void> _runOemBackgroundStep(BuildContext context) async {
    if (!await needsOemBackgroundStep()) return;
    if (!context.mounted) return;
    AppLog.instance.flow("Background", "OEM background restriction — prompting");
    final choice = await _prompt(
      context,
      title: "Allow background on this device",
      body:
          "Your phone manufacturer may restrict apps from running in the background. "
          "Enable autostart / allow background activity for Ghal Bol on the next screen "
          "so messages arrive with the screen off.",
      openLabel: "Open settings",
    );
    if (choice != _ReadinessChoice.open || !context.mounted) return;
    final opened = await openOemBackgroundSettings();
    if (!opened) return;
    await _waitForForegroundReturn();
    await markOemBackgroundStepAcknowledged();
  }

  static Future<_ReadinessChoice?> _prompt(
    BuildContext context, {
    required String title,
    required String body,
    required String openLabel,
  }) {
    return showDialog<_ReadinessChoice>(
      context: context,
      barrierDismissible: false,
      builder: (ctx) => AlertDialog(
        title: Text(title),
        content: Text(body),
        actions: [
          TextButton(
            onPressed: () => Navigator.of(ctx).pop(_ReadinessChoice.later),
            child: const Text("Later"),
          ),
          FilledButton(
            onPressed: () => Navigator.of(ctx).pop(_ReadinessChoice.open),
            child: Text(openLabel),
          ),
        ],
      ),
    );
  }

  /// Waits until the user returns from a system settings screen or permission sheet.
  static Future<void> _waitForForegroundReturn() async {
    final binding = WidgetsBinding.instance;
    if (binding.lifecycleState == AppLifecycleState.resumed) {
      await Future<void>.delayed(const Duration(milliseconds: 350));
      return;
    }
    final completer = Completer<void>();
    late AppLifecycleListener listener;
    listener = AppLifecycleListener(
      onResume: () {
        listener.dispose();
        if (!completer.isCompleted) completer.complete();
      },
    );
    await completer.future.timeout(
      const Duration(minutes: 10),
      onTimeout: () {
        listener.dispose();
      },
    );
    await Future<void>.delayed(const Duration(milliseconds: 350));
  }
}
