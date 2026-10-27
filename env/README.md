# Environment (`env/.env.*`) — optional developer overrides

The app's real configuration lives with the app: `apps/ghalbol/env/development.env` and
`apps/ghalbol/env/production.env`. `apps/ghalbol/build.rs` bakes one of them into the binary:
`GHALBOL_ENV` when set (the scripts set it), otherwise the Cargo profile (`--release` → production).

This folder only holds **local overrides** for desktop development.

| File | In git | Purpose |
|------|--------|---------|
| `env/.env.development.example` | Yes | Template — copy to `.env.development` |
| `env/.env.production.example` | Yes | Template — copy to `.env.production` |
| `env/.env.development` | No | Overrides for development builds run from the workspace root |
| `env/.env.production` | No | Overrides for production builds run from the workspace root |

```bash
cp env/.env.development.example env/.env.development   # then edit, e.g. a local coord URL
```

At startup the app applies, highest first:

1. the process environment (`GHALBOL_COORD_URLS=… cargo run -p ghalbol_app`);
2. `env/.env.<profile>` in the **working directory**, if present;
3. the values baked from `apps/ghalbol/env/<profile>.env`.

Packaged builds (Android, Windows, the Linux tarball) normally have no override file, so they
use the baked values. The app prints the result once on stderr / logcat:
`ghalbol env profile=… src=… GHALBOL_APP_ID=… GHALBOL_COORD_URLS=…`.

`GHALBOL_APP_ID` picks the data directory (`com.ghalbol` or `com.ghalbol.debug`), so an override
file must not change it unless you mean to use another identity store.

**Android:** `./scripts/build_android_app.sh` defaults to **debug** → development env (home coord1).
`--release` / `--aab` select production unless `GHALBOL_ENV` is already exported
(`cargo ghalbol-android-dev` exports `development`).

Coord and delivery **servers** read their own env — see `ghalbol_coord/README.md` and
`ghalbol_delivery/README.md`.
