# Ghal Bol connect invites (URI scheme)

**Format 3 (current emit)** — backward-compatible parse of format 2.

- HTTPS: `https://ghalbol.com/connect/<identity>` — optional `?alias=…` (global alias hint)
- App link: `ghalbol://connect/<identity>` — optional `?alias=…`

`<identity>` is the contact **identity wire** per [MULTI_ALGO.md](MULTI_ALGO.md):

- **Implicit `secp256k1`**: bare 66-char compressed public key hex (no `algorithm:` prefix).
- Other algorithms: `algorithm:hex` in the path segment; `:` is percent-encoded in URLs (`ed25519%3A…`).

Coordination server URL is configured per device (env / native preferences), not in the invite URI.

**Wire map (v3):** `ghalbol.share: "ghal_bol_connect_v1"`, `format_version: 3`, `identity_wire` (full wire), optional `global_alias` (peer-chosen display name on the wire; not unique).

**Wire map (v2, legacy parse):** `format_version: 2`, `public_key_hex` holds the identity wire; optional `peer_alias`.

**Local alias:** device-specific `display_alias` in `contacts_v1.json` only — never authoritative on coord or delivery server. See [IDENTITY.md](IDENTITY.md).

**Implementation:** `ghal_bol_core/src/connect_invite_v1.rs` (canonical encode/decode/verify); Flutter uses native `ghal_bol_core_ffi_build_connect_invite_uri` first, with `invite_uri_codec.dart` as fallback when FFI is unavailable.

**Web handoff (no app):** static pages at `https://ghalbol.com/connect/…` — [WEB_SITE.md](WEB_SITE.md).
