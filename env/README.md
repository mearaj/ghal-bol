# Environment (`env/.env.*`)

| File | In git | Purpose |
|------|--------|---------|
| `env/.env.development.example` | Yes | Template — copy to `.env.development` |
| `env/.env.production.example` | Yes | Template — copy to `.env.production` |
| `env/.env.development` | No | **Debug** builds of `ghalbol_app` (`cargo run` / `cargo build`) |
| `env/.env.production` | No | **Release** builds (`cargo run --release` / packaged release) |

```bash
cp env/.env.development.example env/.env.development
cp env/.env.production.example env/.env.production
```

`ghalbol_app` picks the file from the build profile (`debug_assertions`): development for debug, production for release. Keys already set in the process environment are left unchanged (OS env wins).

**Desktop:** reads `env/.env.*` from the workspace cwd at startup.  
**Android / packaged:** falls back to the committed `env/.env.*.example` (no workspace tree on device). Keep the example URLs in sync with what you want phones to use.

**Android script:** `./scripts/build_android_app.sh` defaults to **debug** (development / coord1). Pass `--release` only for production GCP. A bare script run must not install a release APK over a debug phone build.

On startup the app prints `ghalbol env profile=… GHALBOL_COORD_URLS=…` (stderr / logcat).

Rebuild after editing the file.
