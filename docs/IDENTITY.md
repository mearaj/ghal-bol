# Identity model

Ghal Bol uses a **local-first cryptographic identity** combined with optional premium infrastructure (see [PREMIUM_SERVICES.md](PREMIUM_SERVICES.md)). Communication and payments are intentionally separate.

The coordination server does **not** own identities. Identity exists independently from phone numbers, email, centralized accounts, and payment providers.

---

## What an identity is

Each user has a **secp256k1 public/private keypair**.

| Key | Role |
|-----|------|
| **Private key** | Communication ownership, peer identity ownership, synchronization authority (signing, registration challenges) |
| **Public key** | Peer identifier (66 hex chars), invite links, `ghal_bol_server` lookup |

The system does not require phone numbers, email addresses, or centralized account registration.

**Wire formats (implemented):**

- Public: 66 hex chars (compressed secp256k1)
- Private: 64 hex chars (32-byte secret)
- On disk: `keystore_v1.json` — Argon2id + ChaCha20-Poly1305 (`ghal_bol/src/keystore_v1.rs`)

Storage root (Linux desktop):

| Build | `app_namespace` | Path |
|-------|-----------------|------|
| `flutter run -d linux` (debug) | `com.ghalbol.debug` | `~/.local/share/com.ghalbol.debug/` |
| `flutter run --release -d linux` / shipped bundle | `com.ghalbol` | `~/.local/share/com.ghalbol/` |

Android uses the same `app_namespace` values; debug/release differ by package id (`com.ghalbol.debug` vs `com.ghalbol`). Keystore: `app_flutter/com.ghalbol.debug/` (debug) or `app_flutter/` (release). Contacts/transcript: `app_flutter/ghal_bol/` on both (package id isolates debug vs release).

---

## Identity creation (Flutter)

Two modes on first launch when **no keystore** exists on the device.

### Option 1 — Automatic generation (recommended)

1. User chooses **Create new** and sets an **app password** (required).
2. App generates a keypair locally in Rust (`create_keystore_v1`).
3. Private key is encrypted and written to `keystore_v1.json`.
4. Public key becomes the peer identity (invites, server registration).

**App password is always required** for create, unlock, import, view private key, and delete identity. It encrypts the keystore at rest and is never sent to servers. Optional biometrics may unlock the same blob later, but they do not replace the password.

**Product goal:** instant onboarding with no platform signup — one local password the user chooses once (or on each new device after import).

### Option 2 — Import existing identity (advanced)

1. User chooses **Import key** and pastes a **64-hex private key**, or **Import encrypted keystore backup** (JSON).
2. User sets or enters the **app password** (always required — new encryption for raw-key import, or the backup’s password for keystore import).
3. Imported identity is **fully equivalent** to auto-generated (same APIs, invites, sync).

Use cases: device migration, multi-device setup, recovery from a **Ghal Bol** backup.

If a keystore already exists, import fails until the user **deletes identity** on that device.

**Failed first-time setup:** If create or import fails after the user chose an app password, the app removes any partial `keystore_v1.json` (`ghal_bol_ffi_reset_first_time_identity`) so they can pick a **new password** and try again. The P2P daemon is not allowed to create a keystore before the UI finishes first-time setup (that would block password retry).

### Cryptocurrency wallet keys (not recommended)

Users may import **any valid 64-hex secp256k1 private key**; the app does not forbid wallet keys. **Strongly discouraged:** pasting a private key from Ethereum, Bitcoin, or another cryptocurrency wallet — it controls on-chain funds, is a different purpose than chat, and exposure can drain that wallet. **You are responsible** for what you import.

**Recommended sources:**

- a **64-hex secret exported from Ghal Bol** (Identity → Show private key, after app password), or  
- an **encrypted Ghal Bol keystore backup** from this app.

The first-time UI shows an advisory notice when **Import key** is selected.

---

## Private key visibility

The private key is never shown by default. **View private key** (Identity tab) requires entering the **app password** again, even when the session is already unlocked.

This limits exposure from casual device access or shoulder surfing.

**Flow (implemented):**

1. Identity → **Show private key**
2. App password prompt
3. On success: display 64-hex secret + copy (with warning)

---

## Export and import

Export is **manual, explicit, and user-controlled** — no automatic cloud escrow.

| Export type | Password to view/use | Contents |
|-------------|----------------------|----------|
| **Encrypted keystore backup** | Required to decrypt on another device | Full `keystore_v1.json` JSON (already encrypted) |
| **Private key (hex)** | Required to reveal | 64-hex secret via **Show private key** |

**Warning (show in UI and docs):**

```text
Anyone with this identity backup or private key can fully control your communication identity.
```

**Import (implemented):**

- 64-hex secret + new app password (first-time setup only)
- Encrypted keystore JSON + backup password (first-time setup only)

Restores the same public key, peer identity, and sync authority on the new device.

---

## Ownership philosophy

Ghal Bol identities are:

- local-first
- user-owned
- cryptographic
- infrastructure-independent for direct P2P coordination (see [TRANSPORT.md](TRANSPORT.md))

Servers assist presence and endpoint discovery; they do not issue or own messaging identities.

---

## Implementation reference

| Action | UI | Native FFI |
|--------|-----|------------|
| Create / unlock | Unlock screen | `ghal_bol_ffi_create_or_unlock_identity` |
| Import hex secret | Unlock → Import key | `ghal_bol_ffi_import_identity_from_secret_hex` |
| Import keystore file | Unlock → Import backup | `ghal_bol_ffi_import_keystore_json` |
| Reveal private key | Identity → Show private key | `ghal_bol_ffi_reveal_secret_key_hex` |
| Export backup | Identity → Export backup | `ghal_bol_ffi_export_keystore_json` |
| Delete identity | Unlock / More | `ghal_bol_ffi_delete_keystore` |

Rebuild native after API changes: `sync_ghal_bol_native_for_flutter.sh` (desktop) or `pack_android_workspace_jni_libs.sh` (Android). See [COORDINATION_SERVER.md](COORDINATION_SERVER.md) § Local dev stack.

---

## Security principles

**Local ownership** — Users own identities, private keys, transcripts, and sync state on device.

**Explicit export** — No silent cloud upload of keys; export is user-initiated with clear warnings.

**Secure local storage (today)** — Password-derived key encrypts the identity blob on disk. **Planned:** tighter integration with Android Keystore / iOS Keychain or Secure Enclave for key material wrapping.

**Minimal trust** — `ghal_bol_server` sees registration metadata (public key, endpoints, heartbeats), not private keys or transcript bodies.

**Logging** — `AppLog` redacts `private_key_hex` and similar fields in RPC traces.

---

## Related docs

- [PREMIUM_SERVICES.md](PREMIUM_SERVICES.md) — paid Tier 3 relay, payment rails, membership vs identity
- [GHAL_BOL_URI_SCHEME.md](GHAL_BOL_URI_SCHEME.md) — invites and coordination lookup
- [TRANSPORT.md](TRANSPORT.md) — WAN/LAN dial policy
