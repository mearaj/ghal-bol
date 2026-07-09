import "package:flutter/material.dart";

/// Shared copy for first-time identity setup and sensitive key actions.
abstract final class IdentitySetupCopy {
  static const String appPasswordRequired =
      "An app password is always required. It encrypts your identity on this device and is never sent to any server.";

  static const String identityAlgorithmTitle = "Identity algorithm";

  static const String identityAlgorithmHint =
      "All listed algorithms support P2P chat, calls, and coord on this build.";

  static const String importPrivateKeyWarningTitle =
      "Cryptocurrency wallet keys are not recommended";

  static const String importPrivateKeyWarningBody =
      "You may import any valid 64-hex secp256k1 private key. Ghal Bol does not block your choice — "
      "you are responsible for what you paste.\n\n"
      "Keys from Ethereum, Bitcoin, or other cryptocurrency wallets are strongly discouraged: "
      "they control on-chain funds, serve a different purpose than chat, and exposure can drain that wallet.\n\n"
      "Recommended:\n"
      "• a 64-hex key exported from Ghal Bol (Show private key), or\n"
      "• an encrypted Ghal Bol keystore backup from this app.";

  static const String revealKeyWarning =
      "Anyone with your Ghal Bol private key or backup file plus your app password "
      "can fully control your communication identity. Continue only in a private place.";

  /// Appended when first-time create/import fails and any partial keystore was removed.
  static const String firstTimeRetryHint =
      " Setup did not finish — nothing was saved. You can choose a new app password and try again.";

  /// Shown when unlock succeeds but identity cannot run shipping P2P (non-secp256k1 today).
  static const String nonP2pIdentityBlocked =
      "This identity was created, but P2P chat and calls require secp256k1 on this build. "
      "Delete the identity on this device and create again with secp256k1 (default), "
      "or import a secp256k1 keystore backup.";
}

/// First-launch hints on the identity / unlock screen.
class IdentityFirstSetupBanner extends StatelessWidget {
  const IdentityFirstSetupBanner({super.key, required this.importMode});

  final bool importMode;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Card(
          color: scheme.primaryContainer.withValues(alpha: 0.35),
          child: Padding(
            padding: const EdgeInsets.all(12),
            child: Row(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Icon(Icons.lock_outline, color: scheme.primary, size: 22),
                const SizedBox(width: 10),
                Expanded(
                  child: Text(
                    IdentitySetupCopy.appPasswordRequired,
                    style: Theme.of(context).textTheme.bodySmall,
                  ),
                ),
              ],
            ),
          ),
        ),
        if (importMode) ...[
          const SizedBox(height: 10),
          Card(
            color: scheme.tertiaryContainer.withValues(alpha: 0.65),
            child: Padding(
              padding: const EdgeInsets.all(12),
              child: Row(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Icon(Icons.info_outline, color: scheme.tertiary, size: 26),
                  const SizedBox(width: 10),
                  Expanded(
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Text(
                          IdentitySetupCopy.importPrivateKeyWarningTitle,
                          style: Theme.of(context).textTheme.titleSmall?.copyWith(
                                color: scheme.onTertiaryContainer,
                                fontWeight: FontWeight.w600,
                              ),
                        ),
                        const SizedBox(height: 6),
                        Text(
                          IdentitySetupCopy.importPrivateKeyWarningBody,
                          style: Theme.of(context).textTheme.bodySmall?.copyWith(
                                color: scheme.onTertiaryContainer,
                              ),
                        ),
                      ],
                    ),
                  ),
                ],
              ),
            ),
          ),
        ],
      ],
    );
  }
}
