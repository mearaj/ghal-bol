# Ghal Bol connect invites (URI scheme)

**Format 2 (only):**

- HTTPS: `https://ghalbol.com/connect/<public_key_hex>` — optional `?alias=…` only
- App link: `ghalbol://connect/<public_key_hex>` — optional `?alias=…` only

Coordination server URL is configured per device (env / native preferences), not in the invite URI.

Wire map: `ghalbol.share: "ghal_bol_connect_v1"`, `format_version: 2`, `public_key_hex` only (no multiaddrs; WAN via coordination lookup).

**Implementation:** `ghal_bol/src/connect_invite_v1.rs`, `ghal_bol_ui/lib/invite_uri_codec.dart`, `ghal_bol_ui/lib/ghalbol_connect_invite.dart`.
