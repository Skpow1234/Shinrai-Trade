# Deployment

## What exists today

Shinrai-Trade is documented and CI-verified as a **developer workstation / paper platform**, not a packaged cloud product.

| Piece | In repo? | Notes |
|---|---|---|
| Rust binaries | Yes | `shinrai-md-gateway`, `shinrai-order-gateway` via `cargo build --release` |
| Local Postgres + NATS | Yes | `compose.yaml` — **dependencies only**, not the gateways |
| Dockerfile for gateways | **No** | Not present |
| Kubernetes / Helm / Terraform | **No** | Not present |
| Production runbook | **No** | Local `cargo run` / release binaries only |

Default binds are **localhost** (`127.0.0.1:8080` / `:8081`). Ops and metrics are **unauthenticated unless** `SHINRAI_OG_OPS_TOKEN` (and optionally `SHINRAI_OG_OPS_ALLOWLIST`) are set.

## Local “deploy” (recommended today)

1. Start dependencies:

```bash
docker compose up -d postgres
docker compose up -d nats   # optional outbox bus
cp .env.example .env
```

2. Build release binaries:

```bash
cargo build --workspace --all-features --release
```

3. Run (separate terminals or a process supervisor):

```bash
# Market data
SHINRAI_MD_CLIENTS=… SHINRAI_MD_SYNTH=1 \
  ./target/release/shinrai-md-gateway

# Order gateway (durable)
export SHINRAI_DATABASE_URL=postgres://shinrai:shinrai@127.0.0.1:5432/shinrai
export SHINRAI_RUN_MIGRATIONS=1
export SHINRAI_OG_OPS_TOKEN=…          # require for any non-loopback bind
export SHINRAI_OG_OPS_ALLOWLIST=…      # lock down ops
./target/release/shinrai-order-gateway
```

Or use `--dev` only on a trusted local machine for the paper UI.

## Hardening checklist before any public bind

- Set `SHINRAI_OG_OPS_TOKEN` and a tight `SHINRAI_OG_OPS_ALLOWLIST`
- Prefer short-lived client credentials (`SHINRAI_*_CLIENTS`), not static `SHINRAI_*_TOKENS`
- Keep `SHINRAI_OG_STORE_FAIL_HARD` on when Postgres is enabled
- Do not use compose default DB password outside local Docker
- Put TLS termination in front (Caddy, nginx, cloud LB) — gateways speak plain HTTP today
- Do not treat Phase 5 funds as real money movement

## Practical deployment options (not implemented — how you would)

These are reasonable paths given the current architecture; none are shipped in-tree yet.

### A. Docker Compose (full stack)

Add images for the two binaries alongside existing `postgres` / `nats` services:

- Multi-stage `Dockerfile`: `rust:1.90` build → slim runtime (`debian`/`distroless`)
- Copy release binaries, set `SHINRAI_*` from Compose secrets/env
- Healthchecks against `GET /health`
- Publish only the reverse-proxy port, not Postgres/NATS publicly

### B. Single VM / systemd

- Install Rust toolchain or copy release artifacts from CI
- `docker compose` for Postgres (+ NATS)
- Two systemd units for the gateways with `EnvironmentFile=`
- Optional: Caddy/nginx for TLS + path routing (`/md` vs `/og` or separate hostnames)

### C. Cloud VM + managed Postgres

- Run gateways on a small VM or container service
- Managed Postgres (RDS, Cloud SQL, Neon, etc.) via `SHINRAI_DATABASE_URL`
- Optional managed NATS or skip and keep the log outbox sink
- Wire `SHINRAI_OTEL_ENDPOINT` to your collector

### D. Kubernetes (later)

- Deployments for MD + OG, Service + Ingress, Secret for tokens/DB URL
- StatefulSet or managed Postgres; NATS if you need the bus
- Readiness/liveness on `/health`
- NetworkPolicy so only OG/MD (and ops allowlist path) are reachable

CI already produces confidence via `fmt` / Clippy / cross-OS tests / release build / `test-db`. Extending CI to build and push container images would be the natural next packaging step.

## What not to deploy as-is

- `--dev` or demo tokens (`dev` / `s3cret`) on a public interface
- Open ops (`SHINRAI_OG_OPS_TOKEN` unset) on `0.0.0.0`
- Memory-only OG (no `SHINRAI_DATABASE_URL`) if you need restart durability
- Any assumption of real custody — product scope is paper / licensed-broker paper
