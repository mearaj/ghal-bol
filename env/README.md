# Environment (`env/.env.*`)

| File | In git | Purpose |
|------|--------|---------|
| `env/.env.development.example` | Yes | Template — copy to `.env.development` |
| `env/.env.production.example` | Yes | Template — copy to `.env.production` |
| `env/.env.development` | No | Debug runs of `ghal_bol_app` |
| `env/.env.production` | No | Release runs |

```bash
cp env/.env.development.example env/.env.development
```

`ghal_bol_app` reads `env/.env.development` when the variable is not already set in the process environment. OS environment wins.
