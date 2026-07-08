# Multi-algorithm identity

**Status:** Phase 1–5 **client complete** for `secp256k1`, `ed25519`, `ecdsa-p256`, and **`ml-dsa-65`** (PQ signing + companion libp2p transport key). **P2P chat/calls** run for **all four** algorithms. **DM text**, **call signaling**, and **call audio/video** use **transport KEM v2** after `TransportKemHello`.

**Canonical user-facing overview:** [IDENTITY.md](IDENTITY.md). **This document** is the technical spec for multi-algorithm identity, the identity vs transport/E2E split, and what is implemented vs planned.

---

## Overview and goals

Ghal Bol identities are **cryptographic public keys**. The **`algorithm:` prefix is optional only for `secp256k1`** (bare hex); all other algorithms **require** a prefix. The same identity string is used everywhere the product names a contact or peer: invites, roster, coordination lookup, transcript buckets, and UI.

**Goals:**

- Support multiple public-key algorithms (`secp256k1`, `ed25519`, `ecdsa-p256`, `ml-dsa-65`, …) without free-form algorithm strings.
- **Implicit `secp256k1`:** identity strings with **no** `algorithm:` prefix are defined to use **`secp256k1`** (bare hex = secp256k1 public key).
- **Separate identity from transport/E2E:** the identity key is for discovery and connectivity lookup; message and media confidentiality use **session transport keys** (same general approach as call media today — HKDF + symmetric AES-GCM).
- Stay **transport-agnostic** at the identity layer: validation and storage live in `ghal_bol` with per-algorithm crypto crates. The current P2P stack may use libp2p internally; that is **not** an identity dependency and is not guaranteed long-term.

**Public key is the only wire identity.** Do not use libp2p PeerId (or any other transport handle) as a roster key, invite payload, coord lookup key, or transcript key. Shipping code may still map secp256k1 public key → libp2p PeerId inside the transport layer; that is **legacy transport glue** to isolate, not the multi-algo identity contract.

**Algorithm registry rationale:** The enum lists commonly used public-key algorithms. That set overlaps what many P2P stacks happen to support; it is a **convenience reference only** — Ghal Bol does **not** depend on libp2p for identity parsing, validation, or storage.

---

## Canonical identity format

### Grammar

```text
Identity := [ algorithm ":" ] public_key_hex
```

| Component | Rule |
|-----------|------|
| `algorithm` | **Optional only for `secp256k1`** (omit → implicit `secp256k1`). **Required** for every other algorithm id in the closed enum below. |
| `public_key_hex` | Lowercase hex on the wire. Validated by the named algorithm’s codec in `ghal_bol`. |
| Normalization | Trim whitespace; lowercase hex; single-line. |

### Parsing rules (three cases)

1. **No colon** — algorithm = **`secp256k1`** (implicit by definition); entire string = public key hex.
2. **Colon + known algorithm id** — split on **first** `:` only; left = algorithm, right = public key hex.
3. **Colon + unknown algorithm id** — **reject**. Do not treat the whole string as implicit secp256k1.

### Examples

```text
02a1b2…                     # valid — implicit secp256k1
secp256k1:02a1b2…            # valid — explicit secp256k1
ed25519:9f86d081…            # valid — ed25519
ecdsa-p256:02…               # valid — ecdsa-p256
ml-dsa-65:7c2a…              # valid — ml-dsa-65 (when implemented)
rsa2048:deadbeef…            # invalid — unknown algorithm prefix
ed25519:02a1b2…              # invalid — if hex fails ed25519 codec
```

### What is out of scope for the identity layer

- libp2p `PublicKey`, `PeerId`, protobuf key encoding, or “enable libp2p features per algo.”
- Public key **size** tables in this document — length is defined by each algorithm’s codec at implementation time, not by a global wire table.

### Keystore on disk (`keystore_v1.json`)

Same rule as wire identity: **optional `identity_algorithm` field**; **omitted or empty → implicit `secp256k1`**.

```json
{
  "format": "keystore_v1",
  "kdf": { "salt": "…", "m_cost_kib": 65536, "t_cost": 2, "p_cost": 1 },
  "identity_algorithm": "ed25519",
  "identity_public_key": "<bytes>",
  "identity_nonce": "<12 bytes>",
  "identity_ciphertext": "<encrypted secret>"
}
```

- Keystores with **no** `identity_algorithm` key unlock as **implicit `secp256k1`** (AAD `identity`).
- New secp256k1 keystores also omit the field (canonical on-disk shape for implicit secp256k1).
- Non-secp256k1 keystores set `identity_algorithm` to the wire id (`ed25519`, `ecdsa-p256`, …).

Implementation: [`ghal_bol/src/keystore_v1.rs`](../ghal_bol/src/keystore_v1.rs), [`ghal_bol/src/identity.rs`](../ghal_bol/src/identity.rs).

---

## Algorithm registry (closed enum)

Stable wire constants (kebab-case). Unknown ids → reject when a prefix is present.

| Algorithm id | Status | Role |
|--------------|--------|------|
| `secp256k1` | **Shipping** | **Implicit default** when prefix omitted. Identity, envelope signing, coord registration/lookup, libp2p PeerId derivation. |
| `ed25519` | **Shipping** | Identity, signing, coord, libp2p PeerId derivation, full P2P. |
| `ecdsa-p256` | **Shipping** | Identity, signing, coord, libp2p PeerId derivation, full P2P. |
| `ml-dsa-65` | **Shipping** | Post-quantum **signatures** (ML-DSA-65). Companion ed25519 libp2p transport key; full P2P via transport KEM. |

Implement validation with **standard crypto libraries per algorithm** in `ghal_bol` — **not** `libp2p-identity`.

---

## Identity vs transport / E2E

### Layers

| Layer | Purpose | Uses identity (public key)? |
|-------|---------|----------------------------|
| **Identity** | Who the peer is — invites, contacts, coord register/lookup, roster, transcript keys | Yes — full `Identity` string |
| **Connectivity** | Reach the peer (dial, relay, LAN discovery, streams). Stack is pluggable; libp2p may be used today | Lookup and dial **by public key identity** |
| **Transport / E2E** | DM text bodies, call signaling payloads, call audio/video frames | **No** for payload encryption — session-derived symmetric keys |

### Core rule

The **identity key is not used directly** for message or media payload encryption. It is for **discovery and connectivity** (who to find, who signed an envelope). Confidentiality uses **per-session transport KEM keys** (X25519 ECDH from `TransportKemHello` + HKDF + AES-GCM), with distinct HKDF info for DM text, call signaling, and call media.

Identity may still be used for **signatures** on envelopes (algorithm-specific verify).

### Shipping behavior

| Surface | Implementation |
|---------|----------------|
| DM text | [`transport_kem_v1.rs`](../ghal_bol/src/transport_kem_v1.rs) + `TransportKemHello` → `DM_CIPHER_TRANSPORT_V2` (`0x03`) |
| Call signaling | [`call_sig_v1.rs`](../ghal_bol/src/call_sig_v1.rs) + transport KEM → `CALL_CIPHER_TRANSPORT_V2` (`0x04`) |
| Call audio/video | [`call_media_key.rs`](../ghal_bol/src/call_media_key.rs) — `derive_call_media_keys_from_transport` + HKDF(`call_id`) |
| Envelope signatures | Per-algorithm sign with identity key (auth only) |
| Offline FFI seal | [`offline_seal_v1.rs`](../ghal_bol/src/offline_seal_v1.rs) — encrypt-to-secp256k1 pubkey (`0x10`); auxiliary only |

---

## Where identity appears (impact map)

| Surface | Field / path | Today | Future |
|---------|--------------|-------|--------|
| Connect invite URI | path segment | Full `Identity` wire (bare secp256k1 or `algo:hex`) | format v3 split fields — TBD |
| `ghal_bol_connect_v1` wire map | `public_key_hex` | Full identity wire | optional split `algorithm` field — TBD |
| [`contacts_v1.json`](../ghal_bol/src/contacts_v1.rs) | contact key | Full identity wire | — |
| [`keystore_v1.json`](../ghal_bol/src/keystore_v1.rs) | `identity_algorithm` + encrypted secret | secp256k1 default; ed25519/ecdsa-p256 optional | — |
| DM frames ([`ghal_bol_msg_v1`](GHAL_BOL_DM_MSG_V1.md)) | `sender_public_key_hex` | Identity wire + session ciphertext outbound | Transport session keys for all algorithms |
| Coord `GET /v1/peers/{[algo:]hex}` | path key | Identity wire (URL-encoded) | — |
| Coord agent string | `pk=…` | Full identity wire in identify `agent_version` | — |
| Call signaling | payload ciphertext | Transport KEM (`CALL_CIPHER_TRANSPORT_V2`) | — |
| Call media | peer key input | Transport KEM + HKDF(`call_id`) | — |
| Flutter [`public_key_hex.dart`](../ghal_bol_ui/lib/public_key_hex.dart) | validators | FFI `identityParse` / `identityNormalize` | — |

**Primary code references (today):**

- [`ghal_bol/src/identity.rs`](../ghal_bol/src/identity.rs) — multi-algo wire parse/validate
- [`ghal_bol/src/public_key_util.rs`](../ghal_bol/src/public_key_util.rs) — secp256k1 contact compare (legacy helpers)
- [`ghal_bol/src/identity_ffi.rs`](../ghal_bol/src/identity_ffi.rs) — FFI parse/normalize/same
- [`ghal_bol/src/transport_kem_v1.rs`](../ghal_bol/src/transport_kem_v1.rs) — transport KEM v2 (DM, call sig, call media HKDF)
- [`ghal_bol/src/call_sig_v1.rs`](../ghal_bol/src/call_sig_v1.rs) — call signaling transport seal/open
- [`ghal_bol/src/call_media_key.rs`](../ghal_bol/src/call_media_key.rs) — call media keys from transport KEM
- [`ghal_bol/src/offline_seal_v1.rs`](../ghal_bol/src/offline_seal_v1.rs) — offline encrypt-to-secp256k1 (auxiliary FFI)
- [`ghal_bol/src/symmetric_seal.rs`](../ghal_bol/src/symmetric_seal.rs) — AES-GCM session seal
- [`ghal_bol/src/connect_invite_v1.rs`](../ghal_bol/src/connect_invite_v1.rs) — multi-algo invite validation + URI
- [`ghal_bol/src/identity_sign.rs`](../ghal_bol/src/identity_sign.rs) — per-algorithm envelope signatures
- [`ghal_bol/src/keystore_v1.rs`](../ghal_bol/src/keystore_v1.rs) — multi-algo keystore (optional `identity_algorithm`)
- [`ghal_bol_server/src/agent_pk.rs`](../ghal_bol_server/src/agent_pk.rs) — identify `pk=` identity wire parse

---

## Implementation status

| Item | Status |
|------|--------|
| Implicit `secp256k1` bare-hex identity (contacts, invites, coord, DM envelopes) | **Implemented** |
| `Identity` parse/format/validate (`identity.rs`) | **Implemented** (all four algorithms) |
| Keystore `identity_algorithm` optional field (absent → secp256k1) | **Implemented** |
| Keystore create/unlock secp256k1 (implicit — field omitted on disk) | **Implemented** |
| Keystore create/unlock ed25519 / ecdsa-p256 | **Implemented** |
| Keystore create/unlock `ml-dsa-65` | **Implemented** (32-byte seed) |
| Per-algorithm public key validate/encode (standalone crypto crates) | **Implemented** (`secp256k1` via `secp256k1` crate; `ed25519`/`ecdsa-p256` via Dalek/P-256) |
| Invites / contacts / coord keyed by full `Identity` string (incl. algo prefix) | **Implemented** (`connect_invite_v1`, `contacts_v1`, `coord.rs` URL-encode) |
| FFI `ghal_bol_ffi_identity_*` + Dart `GhalBolFfi.identityParse` | **Implemented** |
| Transport KEM v2 (DM text) | **Implemented** — `DM_CIPHER_TRANSPORT_V2` (`0x03`) after `TransportKemHello` (`transport_kem_v1.rs`, `dm_transport_kem.rs`) |
| DM outbound / inbound text | **Transport v2 only** — requires exchanged hello before send; decrypt `0x03` only |
| Call signaling transport keys | **Implemented** — `CALL_CIPHER_TRANSPORT_V2` (`0x04`) after `TransportKemHello` (`call_sig_v1.rs`, `transport_kem_v1.rs`) |
| Call media transport binding | **Implemented** — `derive_call_media_keys_from_transport` (`call_media_key.rs`) |
| Envelope signatures per identity algorithm | **Implemented** (`identity_sign.rs` — all four algorithms) |
| UI: identity algorithm picker at create | **Implemented** (metadata + validation via `ghal_bol_ffi_identity_*`) |
| UI: QR with prefixed identity | **Implemented** — Rust + FFI; hub/share/chat/bootstrap use `identityWire` |
| Reveal private key shows algorithm | **Implemented** (`reveal_secret_key_hex` + UI) |
| Migration tooling for existing stores | **Implemented** — `list_contacts` normalizes identity wires on read/write |
| Coord register challenge + signature verify | **Implemented** — all four algorithms (`ghal_bol_server/auth.rs`, client `coord_register_auth.rs`) |
| Coord agent `pk=` binding | **Implemented** — identify `agent_version` parses full identity wire (all four algorithms) via `agent_pk.rs` |

**Explicitly not required for the identity layer:** libp2p multi-key identity, PeerId derivation per algorithm, libp2p feature flags for identity.

**P2P chat/calls** run for **all four** identity algorithms (`p2p_ready` for each). **ml-dsa-65** uses a deterministic companion ed25519 libp2p transport key derived from the PQ seed.

---

## Implementation phases (reference)

Phases 0–4 track feature completeness in the repo. **All shipping apps use the current wire** — there is no mixed-version DM decrypt path.

### Phase 1 — Identity parse/validate

- `Identity` parse/format/validate in `ghal_bol` (`identity.rs`); Dart uses **FFI only** (`ghal_bol_ffi_identity_*`).
- Bare hex (no prefix) = implicit **`secp256k1`** by definition; prefixed forms for other algorithms.
- Keystore: optional `identity_algorithm` on disk (absent → implicit secp256k1).

### Phase 2 — Storage and invites

- `contacts_v1`, connect invite, coord paths use **identity string** (public key).

### Phase 3 — Transport / E2E layer

- DM text: transport KEM v2 after `TransportKemHello`.
- Call signaling: session symmetric keys; call media: identity-wire HKDF binding.
- Per-algorithm envelope **signatures**.

### Phase 4 — UI and server

- Algorithm picker at identity creation; QR with optional prefix.
- Coord registration challenges and agent binding per algorithm.

---

## Security

- **Golden rule 7 (E2E):** Payloads remain end-to-end encrypted; target path uses **transport session keys**, not sealing to identity pubkey.
- **Signatures:** Identity key authenticates sender on envelopes; transport keys protect confidentiality.
- **`ml-dsa-65`:** Signing/auth only; PQ does not replace transport-layer confidentiality.
- **Unknown algorithm prefix** when `:` is present → hard reject. Only **missing** prefix implies `secp256k1`.
- **Do not conflate** today’s libp2p transport internals (PeerId, Noise) with this identity spec.

---

## Open questions (TBD)

- Invite URI: prefixed string in path vs `format_version: 3` with separate JSON fields (`algorithm`, `public_key_hex`).
- Coord DB primary key: full identity string vs `(algorithm, public_key_hex)` columns.
- **Transport KEM algorithm** — **DM:** X25519 (`x25519-dalek`). Independent of identity algorithm.
- **Call signaling transport KEM** — TBD (session v1 ships today).

---

## Related docs

- [IDENTITY.md](IDENTITY.md) — user-facing identity model (today: secp256k1)
- [GHAL_BOL_URI_SCHEME.md](GHAL_BOL_URI_SCHEME.md) — connect invite URIs
- [GHAL_BOL_DM_MSG_V1.md](GHAL_BOL_DM_MSG_V1.md) — DM envelopes (transport KEM v2)
- [GHAL_BOL_CALL_NATIVE_V2.md](GHAL_BOL_CALL_NATIVE_V2.md) — call media keys (today: identity-derived)
- [DESIGN.md](DESIGN.md) — layers and E2E overview
- [TRANSPORT.md](TRANSPORT.md) — current connectivity stack (transport only; not identity spec)
