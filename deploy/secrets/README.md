# Compose secrets

File-based secrets for Docker Compose (mounted under `/run/secrets/`).

| File | Used by |
|---|---|
| `postgres_password` | Postgres (`POSTGRES_PASSWORD_FILE`), order-gateway (builds `SHINRAI_DATABASE_URL`) |
| `ops_token` | order-gateway (`SHINRAI_OG_OPS_TOKEN`) |

## Create locally

```bash
./deploy/scripts/init-secrets.sh          # defaults: password shinrai, ops compose-ops
./deploy/scripts/init-secrets.sh --random # URL-safe random values
.\deploy\scripts\init-secrets.ps1
.\deploy\scripts\init-secrets.ps1 -Random
```

Do **not** commit real secret files — only `*.example` is tracked. Changing the Postgres password after the volume exists requires `docker compose down -v` (wipes data).
