# GhalBol — Brand for Communication Products

**GhalBol** (ghalbol.com) — brand family for real-time and asynchronous communication products. Currently shipping **Ghal Bol**, an end-to-end encrypted messenger with voice/video calls and text messaging.

## Workspace Structure (Kumkaj Model)

This workspace follows the Kumkaj pattern: a common workspace shared across multiple apps, with no shared product core. Each app under `apps/` owns all its code independently.

```
ghalbol/
├── Cargo.toml                    # Workspace root — brand-level config
├── README.md                     # This file — brand overview & structure docs
├── docs/                         # Global brand docs (shared doctrine)
│   ├── GHALBOL_BRAND.md          # Brand identity & naming conventions
│   ├── DESIGN.md                 # Architecture design
│   ├── TRANSPORT.md              # Transport policy
│   └── ...                       # Other design docs
├── ghalbol_core/                # Shared core library (identity, crypto, P2P, delivery)
├── ghalbol_coord/               # Coordination server (presence, endpoint discovery)
├── ghalbol_delivery/            # Delivery server (text mailbox)
├── ghalbol_ddns/                # DDNS helper for home deployment
├── apps/                         # Product apps — each brand's products live here
│   └── ghalbol/                  # Ghal Bol Chat (current product)
│       ├── Cargo.toml            # App crate config
│       ├── build.rs              # Env baking (development.env → binary)
│       ├── src/                  # Makepad 2 UI source
│       ├── resources/            # Fonts, icons, assets
│       ├── scripts/              # Per-app dev scripts
│       │   ├── dev.sh            # Development build (hot reload + Tweaker)
│       │   ├── windows-dev.sh    # Windows x86_64 dev package
│       │   └── android-dev.sh    # Android debug APK
│       └── env/                  # Per-app environment files
│           ├── development.env   # Dev coord/delivery URLs
│           └── production.env    # Prod coord/delivery URLs
├── tools/                        # Per-product cargo entrypoints (thin wrappers)
│   ├── ghalbol-dev/              # `cargo ghalbol-dev` → dev.sh
│   ├── ghalbol-android-dev/      # `cargo ghalbol-android-dev` → android-dev.sh
│   └── ghalbol-windows-dev/      # `cargo ghalbol-windows-dev` → windows-dev.sh
├── dist/                         # Build outputs (gitignored)
│   ├── windows/                  # Production Windows MSI package
│   │   ├── package.wxs           # WiX installer manifest
│   │   ├── install.ps1           # PowerShell installer
│   │   ├── uninstall.ps1         # PowerShell uninstaller
│   │   ├── Setup.bat             # Double-click entry point (installer)
│   │   └── Uninstall.bat         # Double-click entry point (uninstaller)
│   └── windows-dev/              # Development Windows package
├── scripts/                      # Shared build/packaging tooling
│   ├── load-ghalbol-env.sh       # Env loader (source into other scripts)
│   ├── desktop-ids.sh            # Desktop install paths, manifest helpers
│   ├── windows-env.sh            # Windows cross-compile setup, resource staging
│   ├── stage-desktop-package.sh  # Package staging (installers + manifests)
│   ├── cargo-target-dir.sh       # Resolve cargo target directory
│   ├── package-windows.sh        # Production Windows build
│   ├── package_linux_release.sh  # Linux x86_64 release tarball
│   ├── build_android_app.sh      # Android APK build
│   └── with-android-env.sh       # Android NDK environment setup
├── vendor/                       # Vendored dependencies (Makepad 2)
├── env/                          # Legacy workspace-level env (for coord/delivery servers)
├── web/                          # ghalbol.com static site + Firebase hosting
└── .gitignore                    # Ignored files pattern
```

## Key Design Decisions

### No Shared Core Library (Like Kumkaj)
- Each `apps/<product>/` owns all its code independently
- `ghalbol_core` is a **library** crate (shared because it serves multiple products), not a product core
- Future GhalBol products under `apps/` will copy from the best-fitting sibling, then diverge
- Reuse is **copy-then-diverge**, duplication is preferable to coupling

### Brand Naming Convention
| Type | Convention | Example |
|------|-----------|---------|
| Brand | CamelCase | GhalBol, ghalbol.com |
| Product name | Title case + space | Ghal Bol |
| Package names | kebab-case | `ghalbol-app` (future), `ghalbol_app` (current) |
| Directory names | snake_case or kebab-case | `ghalbol_core/`, `apps/ghalbol/` |
| App ID prod | `com.ghalbol` | Production Android app id |
| App ID dev | `com.ghalbol.debug` | Debug Android app id |

### Per-App Environment
Each app under `apps/` has its own `env/` directory:
- `development.env` — dev coord/delivery URLs, debug app ID
- `production.env` — prod URLs, production app ID
- `build.rs` bakes the selected env into the binary at compile time

### Development Workflow
```bash
# Desktop development (hot reload + Tweaker via F12)
cargo ghalbol-dev              # → runs apps/ghalbol with --hot --remote

# Windows dev build (cross-compile from Linux with mingw)
cargo ghalbol-windows-dev      # → dist/windows-dev/

# Android dev APK
cargo ghalbol-android-dev

# Production Windows MSI package
./scripts/package-windows.sh  # → dist/windows/

# Production Linux release
./scripts/package_linux_release.sh

# Direct Cargo run (legacy, still works)
cargo run -p ghalbol_app
```

## Adding a New Product

When adding a new communication product under the GhalBol brand:

1. **Create `apps/<product>/`** with its own `Cargo.toml`, `src/`, `resources/`, `env/`, `scripts/`
2. **Create `tools/<product>-dev/` and `tools/<product>-windows-dev/`** as thin wrapper binaries
3. **Add members to workspace Cargo.toml** (`apps/<product>`, `tools/<product>-dev`, `tools/<product>-windows-dev`)
4. **Follow the per-app env pattern**: `env/development.env` + `env/production.env`
5. **Per-app scripts**: `dev.sh`, `windows-dev.sh`, `android-dev.sh`

See [`docs/GHALBOL_BRAND.md`](GHALBOL_BRAND.md) for brand doctrine and naming conventions.

---

**Current product:** Ghal Bol — E2EE messenger (voice/video calls + text via delivery mailbox)
**Infrastructure shared across products:** ghalbol_coord, ghalbol_delivery, ghalbol_ddns
