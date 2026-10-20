# Deployment

## What exists today

| Piece | In repo? | Notes |
|---|---|---|
| Rust binaries | Yes | `shinrai-md-gateway`, `shinrai-order-gateway` |
| Dockerfile | Yes | Multi-stage; targets `md-gateway` / `order-gateway` |
| Compose (deps) | Yes | Postgres + NATS |
| Compose (full stack) | Yes | `--profile full` builds and runs both gateways |
| Kubernetes / Helm / Terraform | No | Not present |
| TLS in-process | No | Terminate at a reverse proxy |

Default host binds for local `cargo run` remain **localhost**. Container images bind `0.0.0.0` inside the network namespace.

## Full stack with Docker Compose

First build compiles the workspace inside Docker (slow once, then cached):

```bash
docker compose --profile full up -d --build
```

| Service | URL / note |
|---|---|
| Market data | `http://127.0.0.1:8080/health` |
| Paper UI | `http://127.0.0.1:8081/ui` — token `full-dev` |
| Ops / metrics | Bearer `compose-ops` (`SHINRAI_OG_OPS_TOKEN`) |
| Postgres | `postgres://shinrai:shinrai@127.0.0.1:5432/shinrai` |
| NATS | `nats://127.0.0.1:4222` |

```bash
docker compose --profile full down
docker compose down -v   # also wipe Postgres volume
```

Build a single image:

```bash
docker build --target md-gateway -t shinrai-md-gateway:local .
docker build --target order-gateway -t shinrai-order-gateway:local .
```

## Local binary “deploy” (no Docker for gateways)

```bash
docker compose up -d postgres nats
cargo build --workspace --all-features --release

SHINRAI_MD_CLIENTS=dev:s3cret:alice SHINRAI_MD_SYNTH=1 \
  ./target/release/shinrai-md-gateway

export SHINRAI_DATABASE_URL=postgres://shinrai:shinrai@127.0.0.1:5432/shinrai
export SHINRAI_RUN_MIGRATIONS=1
export SHINRAI_OG_OPS_TOKEN=…          # required outside --dev
export SHINRAI_OG_OPS_ALLOWLIST=127.0.0.1
./target/release/shinrai-order-gateway
```

`--dev` is for loopback only. Non-loopback + `--dev` requires `SHINRAI_OG_DEV_ALLOW_PUBLIC=1`.

## Ops auth defaults

| Mode | Ops without `SHINRAI_OG_OPS_TOKEN` |
|---|---|
| `--dev` / `SHINRAI_OG_DEV=1` | Open (local demo) |
| `SHINRAI_OG_OPS_OPEN=1` | Open (explicit escape hatch — dangerous) |
| Normal / Compose `full` | **401** `ops_auth_required` |

## Hardening checklist

- Set `SHINRAI_OG_OPS_TOKEN` and a tight `SHINRAI_OG_OPS_ALLOWLIST`
- Prefer short-lived client credentials over static `SHINRAI_*_TOKENS`
- Keep `SHINRAI_OG_STORE_FAIL_HARD` on when Postgres is enabled
- Do not use compose default DB password outside local Docker
- Put TLS in front (Caddy, nginx, cloud LB)
- Do not treat Phase 5 funds as real money movement

## Other options (not packaged)

- **systemd** on a VM with `EnvironmentFile=` and Compose for Postgres
- **Managed Postgres** + gateway containers/VMs + optional NATS
- **Kubernetes** later: Deployments, Ingress, Secrets, probes on `/health`

See [milestones-backlog.md](milestones-backlog.md) for Helm, SBOM, image scanning, and HA follow-ups.
