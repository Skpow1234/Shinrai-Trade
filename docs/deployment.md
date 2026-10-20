# Deployment

## What exists today

| Piece | In repo? | Notes |
|---|---|---|
| Rust binaries | Yes | `shinrai-md-gateway`, `shinrai-order-gateway` |
| Dockerfile | Yes | Multi-stage; targets `md-gateway` / `order-gateway` (both built in CI) |
| Compose secrets | Yes | `deploy/secrets/` + `init-secrets` scripts |
| Compose (deps) | Yes | Postgres + NATS |
| Compose (full stack) | Yes | `--profile full` |
| TLS termination | Yes | Caddy + `compose.tls.yaml` (`--profile tls`) |
| Kubernetes / Helm / Terraform | No | Not present |
| TLS in-process | No | Terminate at Caddy / other proxy |

Default host binds for local `cargo run` remain **localhost**. Container images bind `0.0.0.0` inside the network namespace.

## Secrets (required for Compose)

Compose mounts file secrets (not committed):

| File | Purpose |
|---|---|
| `deploy/secrets/postgres_password` | Postgres + order-gateway DB URL |
| `deploy/secrets/ops_token` | `SHINRAI_OG_OPS_TOKEN` |

```bash
./deploy/scripts/init-secrets.sh           # defaults: shinrai / compose-ops
./deploy/scripts/init-secrets.sh --random  # URL-safe random
.\deploy\scripts\init-secrets.ps1
.\deploy\scripts\init-secrets.ps1 -Random
```

Changing the Postgres password after the data volume exists requires `docker compose down -v` (wipes data). See `deploy/secrets/README.md`.

## Full stack with Docker Compose

```bash
./deploy/scripts/init-secrets.sh
docker compose --profile full up -d --build
```

| Service | URL / note |
|---|---|
| Market data | `http://127.0.0.1:8080/health` |
| Paper UI | `http://127.0.0.1:8081/ui` — token `full-dev` |
| Ops / metrics | Bearer = contents of `deploy/secrets/ops_token` |
| Postgres | user `shinrai`, password from secret file |
| NATS | `nats://127.0.0.1:4222` |

```bash
docker compose --profile full down
docker compose down -v   # also wipe Postgres (+ Caddy) volumes
```

## TLS with Caddy

Self-signed local CA (`tls internal`). Gateways are not published on the host; only Caddy `:80` / `:443`.

Requires Docker Compose **v2.24+** (`!reset` on ports).

```bash
./deploy/scripts/init-secrets.sh
docker compose --profile full --profile tls \
  -f compose.yaml -f compose.tls.yaml up -d --build
```

| URL | Backend |
|---|---|
| `https://og.localhost/ui` | order-gateway |
| `https://md.localhost/health` | md-gateway |

`.localhost` resolves to `127.0.0.1` on modern OS/resolvers. Browsers warn on the first visit — accept/trust the local Caddy CA, or:

```bash
docker compose --profile tls -f compose.yaml -f compose.tls.yaml exec caddy caddy trust
```

Caddyfile: `deploy/Caddyfile`.

Order-gateway still calls MD over the Docker network (`http://md-gateway:8080`), not through Caddy.

## Local binary “deploy” (no Docker for gateways)

```bash
./deploy/scripts/init-secrets.sh
docker compose up -d postgres nats
cargo build --workspace --all-features --release

# Use the same password as deploy/secrets/postgres_password
export SHINRAI_DATABASE_URL=postgres://shinrai:PASSWORD@127.0.0.1:5432/shinrai
export SHINRAI_RUN_MIGRATIONS=1
export SHINRAI_OG_OPS_TOKEN=…   # or match deploy/secrets/ops_token
```

`--dev` is for loopback only. Non-loopback + `--dev` requires `SHINRAI_OG_DEV_ALLOW_PUBLIC=1`.

## Ops auth defaults

| Mode | Ops without `SHINRAI_OG_OPS_TOKEN` |
|---|---|
| `--dev` / `SHINRAI_OG_DEV=1` | Open (local demo) |
| `SHINRAI_OG_OPS_OPEN=1` | Open (explicit escape hatch — dangerous) |
| Normal / Compose `full` | **401** `ops_auth_required` (token from secret file in Compose) |

## Hardening checklist

- Prefer `init-secrets.sh --random` outside throwaway demos
- Set a tight `SHINRAI_OG_OPS_ALLOWLIST` (Compose `full` uses `*` for DX)
- Prefer short-lived client credentials over static `SHINRAI_*_TOKENS`
- Keep `SHINRAI_OG_STORE_FAIL_HARD` on when Postgres is enabled
- Use the TLS overlay (or another reverse proxy) before any public bind
- Do not treat Phase 5 funds as real money movement

## Other options (not packaged)

- **systemd** on a VM with `EnvironmentFile=` and Compose for Postgres
- **Managed Postgres** + gateway containers/VMs + optional NATS
- **Kubernetes** later: Deployments, Ingress, Secrets, probes on `/health`

See [milestones-backlog.md](milestones-backlog.md) for Helm, SBOM, image scanning, and HA follow-ups.
