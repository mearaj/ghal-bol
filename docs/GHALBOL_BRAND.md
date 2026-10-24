# GhalBol — Brand Overview

**GhalBol** (ghalbol.com) — brand for a **family of communication products**. The name "Ghal Bol" means communication in the source language; this brand covers all our real-time and asynchronous communication applications.

## Products under the GhalBol brand

| Product | Type | Status | Description |
|---------|------|--------|-------------|
| **Ghal Bol** (chat) | E2EE messenger | Shipping | Voice/video calls + text via delivery mailbox. Local transcripts, device-owned identity. See [`ghalbol_chat/`](../ghalbol_chat/) |

## Architecture doctrine

- The only shared thing between GhalBol products is the **common workspace**: vendored dependencies, build tooling, and shared infrastructure crates (e.g., `ghalbol_coord`, `ghalbol_delivery`).
- Each product under `ghalbol_app/` owns all of its app/UI code independently.
- Infrastructure crates (`ghalbol_coord`, `ghalbol_delivery`) are shared across products where they serve a common need — but **no shared core product library** between apps.

## Naming convention

| Type | Convention | Example |
|------|-----------|---------|
| Brand | CamelCase (GhalBol) | ghalbol.com, GhalBol family |
| Product name | Title case with space | Ghal Bol |
| Package names | kebab-case | `ghalbol-chat`, `ghalbol-core` |
| Directory names | snake_case or kebab-case | `ghalbol_app/`, `ghalbol_chat/` |
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
├── ghalbol_chat/           # Current product (replaces apps/<app> for now)
│   ├── Cargo.toml          # App crate
│   ├── src/                # App source
│   ├── resources/          # Fonts, icons
│   └── env/                # Per-app env
├── ghalbol_core/          # Shared core library (product logic)
├── ghalbol_coord/         # Coordination server (shared infra)
├── ghalbol_delivery/      # Delivery server (shared infra)
├── ghalbol_ddns/          # DDNS helper (shared infra)
├── apps/                   # Future products live here: apps/<product>/
├── tools/                  # Per-product dev tooling: tools/<product>-dev, tools/<product>-windows-dev
├── dist/                   # Build outputs: dist/windows/, dist/linux/
├── scripts/                # Shared build/packaging tooling
└── vendor/                 # Vendored dependencies (Makepad 2)
```

## Future products

Future communication products under the GhalBol brand will live in `apps/<product>/` following the same structure as `ghalbol_chat/` (or `apps/courier/` from the Kumkaj model). Each product gets its own:
- App crate with Keystore ID
- Environment files (development.env, production.env)
- Resource bundles (fonts, icons)
- Per-app scripts (dev.sh, android-dev.sh, windows-dev.sh)
- Tools entry (`tools/<product>-dev`, `tools/<product>-windows-dev`)