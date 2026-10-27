# GhalBol — Brand Overview

**GhalBol** (ghalbol.com) — brand for a **family of communication products**. The name "Ghal Bol" means communication in the source language; this brand covers all our real-time and asynchronous communication applications.

## Products under the GhalBol brand

| Product | Type | Status | Description |
|---------|------|--------|-------------|
| **Ghal Bol** (chat) | E2EE messenger | Shipping | Voice/video calls + text via delivery mailbox. Local transcripts, device-owned identity. See [`apps/ghalbol/`](../apps/ghalbol/) |

## Architecture doctrine

- GhalBol products share the **common workspace**: build tooling, the `ghalbol_core` library, and infrastructure crates (`ghalbol_coord`, `ghalbol_delivery`, `ghalbol_ddns`).
- Each product under `apps/<product>/` owns all of its app/UI code independently.
- Infrastructure crates (`ghalbol_coord`, `ghalbol_delivery`) are shared across products where they serve a common need — but **no shared core product library** between apps.

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

## Future products

Future communication products under the GhalBol brand will live in `apps/<product>/` following the same structure as `apps/ghalbol/`. Each product gets its own:
- App crate with Keystore ID
- Environment files (development.env, production.env)
- Resource bundles (fonts, icons)
- Per-app scripts (dev.sh, android-dev.sh, windows-dev.sh)
- Tools entry (`tools/<product>-dev`, `tools/<product>-windows-dev`)