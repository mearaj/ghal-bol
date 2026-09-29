# Change Password Flow

**Status:** Implemented.

Changes the app password that encrypts the on-disk keystore. The identity
(public key, algorithm, sync authority) is unchanged — only the at-rest
encryption key differs.

## Entry point

Identity → **Password**
(`ghal_bol_app`, `host::change_password`).

## Steps

1. User taps **Change password**.
2. The password sheet (`host::change_password`)
   prompts for **current password**, **new password**, and **confirm new password**.
3. Client-side checks: current + new non-empty, new == confirm, new != current.
4. Native [`change_password_v1`](../ghal_bol_core/src/storage.rs) verifies the current
   password unlocks the keystore, rewraps the same secret under the new password
   (`create_keystore_v1_from_secret_with_algorithm`), and atomically saves it.
5. The in-process session is reinstalled with the new password so the node keeps running.
6. The user is reminded that **older exported backups still need the OLD password**.

## Ownership

- Rust owns the crypto: unlock old → rewrap → save (`host::change_password`).
- Makepad collects the passwords and shows the result.

## Notes

- Wrong current password fails without modifying the stored keystore.
- The unlocked session stays open across the change; only future unlocks use the new password.
