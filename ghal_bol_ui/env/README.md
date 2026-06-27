# Environment (`env/.env.*`)

| File | In git | Purpose |
|------|--------|---------|
| `env/.env.development.example` | Yes | Template — copy to `.env.development` |
| `env/.env.production.example` | Yes | Template — copy to `.env.production` |
| `env/.env.development` | No (gitignored) | Debug / `flutter run` — bundled via `pubspec.yaml` |
| `env/.env.production` | No (gitignored) | Release builds |

```bash
cp env/.env.development.example env/.env.development   # first time / reset
# edit env/.env.development
# Linux desktop debug: live file is read on each start (no rebuild needed).
# Android / release: rebuild after env changes (bundled asset).
```

CI copies the `.example` files before `dart analyze`. No other magic.
