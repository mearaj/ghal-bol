# GhalBol — Brand Overview

**GhalBol** (ghalbol.com) — brand for a **family of communication products** (a communication operating / management system family), parallel to how **Kumkaj** brands business operating systems with concrete products (Moklanjo, Khaynjo). The name "Ghal Bol" means communication in the source language.

## Products under the GhalBol brand

| Product | Type | Status | Description |
|---------|------|--------|-------------|
| **Ghal Bol** (chat) | E2EE messenger | Shipping | Voice/video calls + text via delivery mailbox. Local transcripts, device-owned identity. See [`apps/ghalbol/`](../apps/ghalbol/) |

## Architecture doctrine

Same workspace pattern as Kumkaj: **brand umbrella + independent products**, with **copy-then-diverge** for UI.

| Layer | Shared? | Notes |
|-------|---------|--------|
| Workspace / Makepad / packaging scripts | Yes | One Cargo workspace |
| `ghalbol_core` + coord/delivery/ddns | Yes | Communication **product logic** and infra — intentional (Kumkaj deliberately has no shared product crate; GhalBol does, because crypto/delivery/calls are the shared communication stack) |
| App UI under `apps/<product>/` | No | Each product owns Splash faces, resources, env, scripts. Reuse faces by copying lessons from siblings / Kumkaj docs — not a shared UI crate |

- Each product under `apps/<product>/` owns all of its app/UI code independently.
- UI lessons (intent→face, Overlay selection, 720 chrome) live in [`MAKEPAD_UI.md`](MAKEPAD_UI.md) and [`.cursor/rules/makepad-layout-faces.mdc`](../.cursor/rules/makepad-layout-faces.mdc).

## Naming convention

| Type | Convention | Example |
|------|-----------|---------|
| Brand | CamelCase (GhalBol) | ghalbol.com, GhalBol family |
| Product name | Title case with space | Ghal Bol |
| Package names | kebab-case | `ghalbol-chat`, `ghalbol-core` |
| Directory names | snake_case or kebab-case | `apps/ghalbol/`, `ghalbol_core/` |
| App IDs (production) | `com.ghalbol.<product>` | `com.ghalbol`, `com.ghalbol.debug` |
| App IDs (development) | `com.ghalbol.<product>.debug` | `com.ghalbol.debug` |

## Workspace layout

```
ghalbol-workspace/          # Brand root
├── Cargo.toml              # Workspace root
├── README.md               # Brand overview
├── docs/                   # Global brand docs
│   ├── GHALBOL_BRAND.md    # This file — brand doctrine
│   ├── DESIGN.md           # Architecture design
│   ├── TRANSPORT.md        # Transport policy
│   └── ...                 # Other global docs
├── ghalbol_core/          # Shared core library (product logic)
├── ghalbol_coord/         # Coordination server (shared infra)
├── ghalbol_delivery/      # Delivery server (shared infra)
├── ghalbol_ddns/          # DDNS helper (shared infra)
├── apps/                   # Products: apps/ghalbol/ (Ghal Bol chat) today
│   └── ghalbol/            # App crate `ghalbol_app`: src/, resources/, env/, scripts/
├── tools/                  # Per-product dev tooling: tools/<product>-dev, tools/<product>-windows-dev
├── dist/                   # Build outputs: dist/windows/, dist/windows-dev/ (Linux tarball → web/downloads/)
└── scripts/                # Shared build/packaging tooling
```

## Visual identity (Ghal Bol chat)

Palette is sampled from [`docs/assets/app_icon.png`](assets/app_icon.png): cool shell `#EEF2F8`, electric blue primary `#0183FB`, navy chrome `#10236D`. Documented in [`MAKEPAD_UI.md`](MAKEPAD_UI.md) § Palette. Future products may diverge (Kumkaj lesson: each product gets the design that serves *its* users).

## Future products

Future communication products under the GhalBol brand will live in `apps/<product>/` following the same structure as `apps/ghalbol/`. Each product gets its own:
- App crate with Keystore ID
- Environment files (development.env, production.env)
- Resource bundles (fonts, icons, optional `resources/nav/` marks)
- Per-app scripts (dev.sh, android-dev.sh, windows-dev.sh)
- Tools entry (`tools/<product>-dev`, `tools/<product>-windows-dev`)
- Its own Splash face copy (intent→face vocabulary recommended; palette may differ)