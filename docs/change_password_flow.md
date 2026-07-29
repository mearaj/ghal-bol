# Change Password Flow

**Status:** Implemented.

Changes the app password that encrypts the on-disk keystore. The identity
(public key, algorithm, sync authority) is unchanged — only the at-rest
encryption key differs.

## Entry point

Identity tab → **Backup & private key** → **Change password**
([`chat_hub_screen.dart`](../ghal_bol_ui/lib/chat_hub_screen.dart) `_identityBody`).
Shown only when native key management exposes the change-password symbol
(`GhalBolFfi.isChangePasswordAvailable`).

## Steps

1. User taps **Change password**.
2. Dialog ([`showChangePasswordDialog`](../ghal_bol_ui/lib/identity_key_management.dart))
   prompts for **current password**, **new password**, and **confirm new password**.
3. Client-side checks: current + new non-empty, new == confirm, new != current.
4. Native [`change_password_v1`](../ghal_bol_core/src/storage.rs) verifies the current
   password unlocks the keystore, rewraps the same secret under the new password
   (`create_keystore_v1_from_secret_with_algorithm`), and atomically saves it.
5. On success the cached daemon credential ([`SessionCredentials`](../ghal_bol_ui/lib/session_credentials.dart))
   is updated so the out-of-process P2P daemon can re-unlock after a restart.
6. The user is reminded that **older exported backups still need the OLD password**
   and offered to export a fresh backup immediately.

## Ownership

- Rust owns the crypto: unlock old → rewrap → save (`ghal_bol_core`).
- FFI: `ghal_bol_core_ffi_change_password` → `GhalBolFfi.changePassword`.
- Flutter is UI only (prompts, validation, re-export reminder).

## Notes

- Wrong current password fails without modifying the stored keystore.
- The daemon's live in-memory session stays unlocked across the change; only
  future unlocks use the new password.
