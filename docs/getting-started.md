# Getting started

## Prerequisites

- [Rustup](https://rustup.rs/). Opening the repo installs **1.90.0** plus `clippy` and `rustfmt` from `rust-toolchain.toml`.
- Optional WebSocket clients: [`websocat`](https://github.com/vi/websocat) (`cargo install websocat`) or `npx wscat`.
- Optional [Docker](https://www.docker.com/get-started/) / Docker Compose for local PostgreSQL and NATS. **Not required** for `cargo test --workspace` — dual-write tests skip without `SHINRAI_DATABASE_URL`.

## PostgreSQL and NATS

Dev Postgres backs `shinrai-store` (orders, ledger, audit, transactional outbox). With `SHINRAI_DATABASE_URL` set, the order gateway **write-through** persists before ack and **hydrates** `PaperEngine` (including working orders) on restart. Without the URL, the order gateway stays in-memory only.

Optional NATS (`SHINRAI_NATS_URL`) publishes outbox rows to `{prefix}.{topic}` then marks them published (at-least-once). Without NATS the publisher uses the log sink only.

```bash
# File secrets for Compose (gitignored) — defaults match local docs
./deploy/scripts/init-secrets.sh          # or: .\deploy\scripts\init-secrets.ps1

docker compose up -d postgres
docker compose up -d nats          # optional event bus

# postgres://shinrai:<deploy/secrets/postgres_password>@127.0.0.1:5432/shinrai
# nats://127.0.0.1:4222

docker compose ps
docker compose down                # stop
docker compose down -v             # stop and delete the data volume
```

Full stack + optional TLS: see [Deployment](deployment.md).

```bash
cp .env.example .env
# Align SHINRAI_DATABASE_URL password with deploy/secrets/postgres_password
```

| Variable | Meaning |
|---|---|
| `SHINRAI_DATABASE_URL` | Postgres URL (dev default matches `compose.yaml`) |
| `SHINRAI_DB_POOL_SIZE` | Connection pool size (default 5) |
| `SHINRAI_RUN_MIGRATIONS` | When `1`/`true`, order gateway runs migrations on start (dev) |

Store and order-gateway dual-write tests skip when the URL is unset. With Postgres running:

```bash
SHINRAI_DATABASE_URL=postgres://shinrai:shinrai@127.0.0.1:5432/shinrai \
  cargo test -p shinrai-store --all-features

SHINRAI_DATABASE_URL=postgres://shinrai:shinrai@127.0.0.1:5432/shinrai \
  cargo test -p shinrai-order-gateway --all-features --test dual_write -- --test-threads=1
```

CI host tests stay Postgres-free; the **`test-db`** job runs store, dual-write, and chaos tests against a Postgres service. Protect the branch with the single required check named **CI**.

## Build

```bash
cargo build --workspace --all-features
cargo build --workspace --all-features --release

cargo build -p shinrai-md-gateway
cargo build -p shinrai-order-gateway
cargo build -p shinrai-md-gateway --release
cargo build -p shinrai-order-gateway --release
```

Release binaries: `target/release/shinrai-md-gateway` and `target/release/shinrai-order-gateway` (`.exe` on Windows).

## Run (quick)

Market-data gateway (fail-closed WebSocket with empty token table):

```bash
cargo run -p shinrai-md-gateway
curl http://127.0.0.1:8080/health
```

Paper order gateway + UI:

```bash
cargo run -p shinrai-order-gateway -- --dev
# → http://127.0.0.1:8081/ui  (token: dev)

./scripts/dev-ui.sh          # macOS / Linux / Git Bash
.\scripts\dev-ui.ps1         # PowerShell
scripts\dev-ui.cmd           # cmd.exe
```

See [Endpoints](endpoints.md), [Configuration](configuration.md), and [Local demo](demo.md) for full details.
