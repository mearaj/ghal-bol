# Ghal Bol connect invites (URI scheme)

**Format 2 (only):**

- HTTPS: `https://ghalbol.com/connect/<identity>` — optional `?alias=…` only
- App link: `ghalbol://connect/<identity>` — optional `?alias=…` only

`<identity>` is the contact **identity wire** per [MULTI_ALGO.md](MULTI_ALGO.md):

- **Implicit `secp256k1`**: bare 66-char compressed public key hex (no `algorithm:` prefix).
- Other algorithms: `algorithm:hex` in the path segment; `:` is percent-encoded in URLs (`ed25519%3A…`).

Coordination server URL is configured per device (env / native preferences), not in the invite URI.

Wire map: `ghalbol.share: "ghal_bol_connect_v1"`, `format_version: 2`, `public_key_hex` field holds the full identity wire (bare secp256k1 or prefixed).

**Implementation:** `ghal_bol/src/connect_invite_v1.rs` (canonical encode/decode/verify); Flutter uses native `ghal_bol_ffi_build_connect_invite_uri` first, with `invite_uri_codec.dart` as fallback when FFI is unavailable.

**Web handoff (no app):** static pages at `https://ghalbol.com/connect/…` — [WEB_SITE.md](WEB_SITE.md).
