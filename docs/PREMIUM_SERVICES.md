# Premium services and payments

**Status: product architecture — billing and Tier 3 relay are not implemented in this repo yet.**

Ghal Bol’s **core messaging** (Tier 1 direct sync, optional Tier 2 peer relay) stays independent of payment systems and blockchain infrastructure. Optional **premium infrastructure** (Tier 3 backup relay) is a separate product layer.

See [COMMUNICATION_TIERS.md](COMMUNICATION_TIERS.md) for transport tiers. See [IDENTITY.md](IDENTITY.md) for cryptographic identity (create, import, export).

---

## Overview

Design goals:

- user-owned [identity](IDENTITY.md)
- peer-to-peer communication first
- minimal centralized dependence for free tiers
- optional premium reliability when users want it
- simple payment experience without turning the app into a wallet

Premium does **not** change who owns transcripts or private keys. It only changes whether encrypted blobs may be held temporarily on Ghal Bol-operated infrastructure.

---

## What premium may include (planned)

Examples aligned with **Tier 3** in [COMMUNICATION_TIERS.md](COMMUNICATION_TIERS.md):

- durable backup relay (encrypted blobs, bounded TTL)
- longer retention windows for delayed sync
- stronger offline delivery assistance
- cloud-assisted synchronization when peers are not online together

**Not in scope for premium:**

- owning user identities
- permanent centralized transcript archives
- replacing E2E encryption with server-readable mail

`ghal_bol_server` **today** is Tier 1 coordination only (register, heartbeat, peer lookup). It is **not** Tier 3 billing or blob storage.

---

## Separation: communication identity vs payment identity

| | Communication identity | Payment identity |
|--|------------------------|------------------|
| **Purpose** | Who you are on the network | How you pay for optional services |
| **Based on** | secp256k1 messaging keypair | External wallet / payment rail |
| **Used for** | Signatures, invites, sync, registration | Invoices, entitlement, subscription |

**Payment wallets are not messaging identities.** This keeps privacy boundaries clear and avoids coupling key rotation to wallet addresses.

---

## Premium membership model (planned)

Entitlement should **not** be permanently welded to a single public key.

Users may:

- rotate or import identities ([IDENTITY.md](IDENTITY.md))
- migrate devices
- restore from backup

…without irreversibly losing premium access, as long as entitlement is bound to a **separate account or subscription record** (design TBD), not to “this 66-hex key forever.”

Treat as two concerns:

1. **Communication identity** — cryptographic, local-first  
2. **Infrastructure entitlement** — optional, revocable, payment-backed  

---

## Crypto payment philosophy

Cryptocurrency is intended only as a **payment rail** for optional premium services.

Ghal Bol is **not**:

- a crypto messenger
- a blockchain social network
- a wallet platform
- a token ecosystem

Core messaging must work with **no** chain, **no** token, and **no** in-app wallet.

---

## Payment experience (planned)

Simple, wallet-agnostic flow:

1. User selects a premium offering in the app.
2. Backend generates an **invoice** (amount, reference, expiry).
3. App shows:
   - deposit address (or payment link)
   - amount
   - supported chain / asset
4. User pays with **any external wallet** (no embedded wallet required).
5. Server detects settlement (chain watcher or provider webhook).
6. Premium entitlement activates for the linked account.

The app should **not** require:

- in-app wallet seed management
- blockchain login
- smart-contract interaction from the chat UI

---

## Supported chains (candidates)

Architecture may support multiple networks for flexibility and fee choice, for example:

- Ethereum
- Polygon
- Tron
- Solana
- BNB Chain

**Stablecoins preferred** for predictable pricing and ops (e.g. USDC, USDT on chosen chains).

Exact chain list and custody model are product/ops decisions, not fixed in code yet.

---

## Security and trust (premium layer)

Even when Tier 3 exists:

- message payloads remain **end-to-end encrypted**
- servers store **opaque blobs**, not transcripts of record
- [identity](IDENTITY.md) and transcript truth stay on peers
- payment metadata is separate from DM content

Servers may assist:

- coordination (Tier 1, shipped)
- payment verification (planned)
- premium relay storage (planned)

They must not become owners of identities or long-term chat history.

---

## Implementation map (repo)

| Piece | Status |
|-------|--------|
| Tier 1 `ghal_bol_server` | Shipped |
| Tier 3 backup relay service | Not started |
| Invoice / payment detection API | Not started |
| Flutter premium UI / paywall | Not started |
| Entitlement store (decoupled from pubkey) | Not started |

When Tier 3 is built, document APIs here and link from [ghal_bol_server/README.md](../ghal_bol_server/README.md) only if the same binary gains premium features — otherwise a separate service crate is likely.

---

## Long-term vision

Combine:

- mainstream usability (auto identity, optional import/export)
- cryptographic ownership
- P2P-first communication
- lightweight free infrastructure
- optional paid reliability

…without requiring users to be blockchain experts, wallet managers, or cryptography specialists. Decentralized **ownership** should feel simple; premium is an optional convenience layer, not the product definition.
