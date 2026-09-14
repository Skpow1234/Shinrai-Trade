# Shinrai-Trade

[![CI](https://github.com/Skpow1234/Shinrai-Trade/actions/workflows/ci.yml/badge.svg)](https://github.com/Skpow1234/Shinrai-Trade/actions/workflows/ci.yml)
[![Rust](https://img.shields.io/badge/rust-1.90.0-orange.svg)](https://www.rust-lang.org/)
[![Tokio](https://img.shields.io/badge/tokio-async-informational.svg)](https://tokio.rs/)
[![Axum](https://img.shields.io/badge/axum-0.8-blue.svg)](https://github.com/tokio-rs/axum)
[![Proptest](https://img.shields.io/badge/proptest-invariants-yellow.svg)](https://github.com/proptest-rs/proptest)
[![License: MIT](https://img.shields.io/badge/license-MIT-green.svg)](https://opensource.org/licenses/MIT)

A Rust trading platform workspace: domain correctness first, then a read-only market-data path, then authenticated paper orders. It is **not** an exchange and does **not** hold customer funds in production. Phase 3 adds a paper order gateway (pre-trade risk → OMS → simulated venue → ledger).

The intended product is a trading application connected to a licensed broker or venue. Phase 1 proved money, instruments, an order state machine, a double-entry ledger, and a paper loop **in-process**. Phase 2 is a read-only market-data gateway (normalize, gap/degrade, bounded WebSocket fanout, historical bars/trades, short-lived auth). Phase 3 wires **paper orders** through an HTTP gateway with pre-trade risk.

## Tech

| Area | Choice |
|---|---|
| Language / edition | Rust 1.90.0 (`rust-toolchain.toml`), edition 2021, workspace resolver 2 |
| Async / HTTP | Tokio, Axum 0.8 (HTTP + WebSocket) |
| Serialization | Serde / `serde_json` (JSON at the client and vendor edges only) |
| Money / prices | Integer minor units, ticks, and lots — **no `f32`/`f64` on money or prices** |
| Tests | Colocated unit tests, crate `tests/`, Proptest on the paper loop |
| CI | GitHub Actions: `fmt`, Clippy `-D warnings`, tests on Ubuntu / Windows / macOS, release build |

Domain crates stay independent of Axum, databases, and a specific venue. The gateway is a thin I/O edge around `shinrai-md-fanout`.

## Layout

```text
Shinrai-Trade/
├── crates/domain/           # money, instruments, ledger, orders, market-data, paper, risk
├── crates/protocols/       # Coinbase adapter + client fanout (no sockets in fanout)
├── crates/services/        # market-data-gateway, order-gateway (Axum binaries)
├── crates/infrastructure/  # shinrai-store (Postgres), shinrai-telemetry (tracing/OTLP)
├── crates/testing/         # exchange simulator
├── compose.yaml            # optional Postgres for durable store
├── .env.example            # SHINRAI_DATABASE_URL and related placeholders
└── .github/workflows/ci.yml
```

| Crate | Role |
|---|---|
| `shinrai-money` | Currencies and integer money |
| `shinrai-instruments` | Instrument master, tick/lot grid, aliases (`AAPL`, `ESZ5`, `BTC-USD`) |
| `shinrai-ledger` | Double-entry journal |
| `shinrai-orders` | Order FSM + idempotent store |
| `shinrai-market-data` | Tick journal, OHLCV, L2 book, replay |
| `shinrai-paper` | Paper loop: reserve → simulated venue → fill → settle |
| `shinrai-risk` | Pre-trade checks: buying power, limits, kill switches |
| `shinrai-audit` | Append-only audit trail for the trading path |
| `shinrai-portfolio` | Cash, positions, mark-to-market P&L snapshots |
| `shinrai-md-protocol` | Coinbase Exchange decode, raw journal, feed supervisor |
| `shinrai-md-fanout` | Sessions, authn, bounded queues, heartbeats |
| `shinrai-md-gateway` | `GET /health`, bars/trades/quotes, WebSocket |
| `shinrai-order-gateway` | Orders, portfolio, audit, reconciliation, metrics |
| `shinrai-execution` | Execution reports + `ExecutionVenue` trait; in-process sandbox broker |
| `shinrai-store` | PostgreSQL: orders, ledger, audit, transactional outbox |
| `shinrai-telemetry` | Process tracing + optional OTLP export (`SHINRAI_OTEL_ENDPOINT`) |
| `shinrai-exchange-simulator` | Scripted venue for paper tests (`ExecutionVenue`) |

## Prerequisites

- [Rustup](https://rustup.rs/). Opening the repo installs **1.90.0** plus `clippy` and `rustfmt` from `rust-toolchain.toml`.
- Optional WebSocket clients for the demo: [`websocat`](https://github.com/vi/websocat) (`cargo install websocat`) or `npx wscat`.
- Optional [Docker](https://docs.docker.com/get-docker/) / Docker Compose for local PostgreSQL . **Not required** for `cargo test --workspace` today — dual-write tests skip without `SHINRAI_DATABASE_URL`.

## PostgreSQL

Dev Postgres for `shinrai-store` (orders, ledger, audit, transactional outbox). Domain crates stay free of Docker. With `SHINRAI_DATABASE_URL` set, the order gateway **write-through** persists to Postgres before ack and **hydrates** `PaperEngine` (including working orders) on restart. Without the URL, OG stays in-memory only.

```bash
# Start (healthcheck: pg_isready)
docker compose up -d postgres

# Connection string (same as `.env.example`)
# postgres://shinrai:shinrai@127.0.0.1:5432/shinrai

docker compose ps
docker compose down       # stop
docker compose down -v    # stop and delete the data volume
```

Copy env placeholders:

```bash
cp .env.example .env
```

| Variable | Meaning |
|---|---|
| `SHINRAI_DATABASE_URL` | Postgres URL (dev default matches `compose.yaml`) |
| `SHINRAI_DB_POOL_SIZE` | Connection pool size (default 5) |
| `SHINRAI_RUN_MIGRATIONS` | When `1`/`true`, order gateway runs migrations on start (dev) |

Store and OG dual-write tests skip when the URL is unset. With Postgres running:

```bash
SHINRAI_DATABASE_URL=postgres://shinrai:shinrai@127.0.0.1:5432/shinrai \
  cargo test -p shinrai-store --all-features

SHINRAI_DATABASE_URL=postgres://shinrai:shinrai@127.0.0.1:5432/shinrai \
  cargo test -p shinrai-order-gateway --all-features --test dual_write -- --test-threads=1
```

CI matrix `cargo test --workspace` stays host-only; a dedicated **`test-db`** job runs store + OG dual-write tests against a Postgres service.

## Build

Debug (all workspace members):

```bash
cargo build --workspace --all-features
```

Release (same as CI):

```bash
cargo build --workspace --all-features --release
```

One binary:

```bash
cargo build -p shinrai-md-gateway
cargo build -p shinrai-order-gateway
cargo build -p shinrai-md-gateway --release
cargo build -p shinrai-order-gateway --release
```

The release binaries are `target/release/shinrai-md-gateway` and `target/release/shinrai-order-gateway` (`.exe` on Windows).

## Run

### Market-data gateway

With **no** env vars it binds `127.0.0.1:8080` and **rejects every WebSocket** (fail-closed: empty token table).

```bash
cargo run -p shinrai-md-gateway
```

Health (no auth):

```bash
curl http://127.0.0.1:8080/health
```

Expect `{"status":"ok","service":"market-data-gateway","feed":"none|synth|coinbase"}`.

### Authentication

Short-lived access tokens (default **60s**) and rotating refresh tokens.

| Env | Meaning |
|---|---|
| `SHINRAI_MD_CLIENTS` | `client_id:secret:subject,...` — issue via `POST /v1/auth/token` |
| `SHINRAI_MD_TOKENS` | `token:subject,...` — non-expiring bootstrap access tokens (local demos) |
| `SHINRAI_MD_ACCESS_TTL` | Access lifetime in seconds (default `60`) |
| `SHINRAI_MD_REFRESH_TTL` | Refresh lifetime in seconds (default `3600`) |
| `SHINRAI_MD_SYNTH` | `1` / `true` — synthetic BTC-USD publisher (local demo) |
| `SHINRAI_MD_COINBASE` | `1` / `true` — live Coinbase Exchange public WS + REST snapshots (BTC-USD). Wins if both synth and coinbase are set. |

```bash
# Issue
curl -s -X POST http://127.0.0.1:8080/v1/auth/token \
  -H 'content-type: application/json' \
  -d '{"grant_type":"client_credentials","client_id":"dev","client_secret":"s3cret"}'

# Refresh (old refresh becomes invalid; reuse revokes the token family)
curl -s -X POST http://127.0.0.1:8080/v1/auth/token \
  -H 'content-type: application/json' \
  -d '{"grant_type":"refresh_token","refresh_token":"<refresh>"}'

# Revoke access or refresh (sessions using that access die on the next hub clock tick, ≪ 60s)
curl -s -X POST http://127.0.0.1:8080/v1/auth/revoke \
  -H 'content-type: application/json' \
  -d '{"token":"<access_or_refresh>"}'
```

### Historical REST (auth required)

Same access token as WebSocket (`Authorization: Bearer …` or `?token=`).

```bash
# Minute bars (also: 1s / 1h / 1d, or raw seconds e.g. interval=60)
curl "http://127.0.0.1:8080/v1/bars?symbol=BTC-USD&interval=1m&limit=10&token=$ACCESS"

# Trade prints; pass next_cursor from the response for the next page
curl "http://127.0.0.1:8080/v1/trades?symbol=BTC-USD&limit=50&token=$ACCESS"
```

Optional filters: `start` / `end` (logical or Unix seconds matching the store). Prices and sizes are scaled integers (`*_scaled`, `*_lots`). Gateway startup seeds ~120 synthetic BTC-USD trades so these endpoints work without a publisher. With `SHINRAI_MD_SYNTH=1`, additional synthetic prints append to the archive. With `SHINRAI_MD_COINBASE=1`, the gateway opens the public Coinbase Exchange WebSocket (BTC-USD), runs `FeedSupervisor` (gap → REST book snapshot), and fans out ticks / degrade / book-ready to clients.

### Order gateway (paper trading)

Binds `127.0.0.1:8081` by default. Requires auth and a subject → account mapping.

When `SHINRAI_DATABASE_URL` is set, Postgres is **required write-through**: submit/cancel ack only after a successful transactional persist. Persist failure returns **503** `persist_failed` and engages the global kill switch. Startup hydrates OMS/ledger/audit/positions and reinflates working-order reserves + in-process venue state.

| Env | Meaning |
|---|---|
| `SHINRAI_OG_BIND` | Listen address (default `127.0.0.1:8081`) |
| `SHINRAI_OG_CLIENTS` | `client_id:secret:subject,...` |
| `SHINRAI_OG_TOKENS` | `token:subject,...` — bootstrap static tokens |
| `SHINRAI_OG_ACCOUNTS` | `subject:account_id,...` — maps auth subject to ledger account |
| `SHINRAI_OG_DEPOSITS` | `account_id:usd_major,...` — bootstrap paper cash |
| `SHINRAI_OG_ACCESS_TTL` / `SHINRAI_OG_REFRESH_TTL` | Token lifetimes (same semantics as MD gateway) |

```bash
SHINRAI_OG_TOKENS=dev:trader \
SHINRAI_OG_ACCOUNTS=trader:1 \
SHINRAI_OG_DEPOSITS=1:10000 \
cargo run -p shinrai-order-gateway

curl -s -X POST "http://127.0.0.1:8081/v1/orders?token=dev" \
  -H 'content-type: application/json' \
  -d '{"client_order_id":"o1","symbol":"AAPL","side":"Buy","qty":10,"price":10000}'
```

Pre-trade risk runs before the OMS. Insufficient buying power returns **422** with `"code":"insufficient_buying_power"`. Duplicate `client_order_id` for the same account is idempotent (returns the existing order).

| Env | Meaning |
|---|---|
| `SHINRAI_OG_MARKS` | Bootstrap marks `SYMBOL:price_scaled,...` |
| `SHINRAI_OG_MD_URL` | MD gateway base URL for `use_live_marks=1` on portfolio |
| `SHINRAI_OG_MD_TOKEN` | Access token when calling the MD gateway |
| `SHINRAI_OG_STUCK_AGE_SECS` | Pending OMS age before “stuck” (default `5`) |
| `SHINRAI_OG_VENUE` | `sim` (default) or `sandbox` (in-process broker sandbox) |
| `SHINRAI_LOG` | `tracing` filter (falls back to `RUST_LOG`, default `info`) |
| `SHINRAI_OTEL_ENDPOINT` | OTLP/HTTP base URL (e.g. `http://127.0.0.1:4318`); also accepts `OTEL_EXPORTER_OTLP_ENDPOINT` |
| `SHINRAI_OUTBOX_POLL_MS` | Outbox publisher poll interval when Postgres is enabled (default `1000`) |

Order-path spans (`order.submit`, `order.cancel`) carry `account_id`, `client_order_id` / `order_id`, and `outcome`. HTTP requests get a `tower-http` trace layer. Without an OTLP endpoint, spans still appear on stderr via the fmt layer.

Additional authenticated routes:

```bash
# List orders
curl "http://127.0.0.1:8081/v1/orders?token=dev"

# Portfolio (stored fill marks by default; optional live MD quotes)
curl "http://127.0.0.1:8081/v1/portfolio?token=dev&use_stored_marks=1"
curl "http://127.0.0.1:8081/v1/portfolio?token=dev&use_live_marks=1"

# Append-only audit trail (paginate with after_seq)
curl "http://127.0.0.1:8081/v1/audit?token=dev"

# OMS vs simulated-venue reconciliation
curl "http://127.0.0.1:8081/v1/reconciliation?token=dev"
```

Unauthenticated local ops (do not expose publicly):

```bash
# Counters + OMS status histogram + stuck pending + ledger/recon health
curl http://127.0.0.1:8081/v1/metrics

# Stuck PendingNew / PendingCancel / PendingReplace
curl "http://127.0.0.1:8081/v1/ops/stuck-orders?max_age_secs=5"

# Minimal HTML dashboard (polls /v1/metrics)
open http://127.0.0.1:8081/v1/ops   # or browse to that URL
```

## Test

Same flags as CI:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

Format in place:

```bash
cargo fmt --all
```

Single crate / filter:

```bash
cargo test -p shinrai-paper --all-features
cargo test -p shinrai-md-gateway --test auth
cargo test -p shinrai-order-gateway --test orders
cargo test -p shinrai-order-gateway --test portfolio
cargo test -p shinrai-order-gateway --test live_marks
cargo test -p shinrai-store --all-features
cargo test -p shinrai-audit --lib
cargo test -p shinrai-risk --lib
cargo test -p shinrai-md-gateway --test health
cargo test -p shinrai-md-fanout --lib overflow_drops_oldest
cargo test -p shinrai-paper --test proptest_invariants
```

CI does **not** open a live Coinbase socket. Vendor tests use recorded JSON under `crates/protocols/market-data/tests/fixtures/`. Live portfolio marks are covered by `live_marks` (local MD gateway on `127.0.0.1:0`, no external network).

## Local demo

**Recommended:** client credentials + short-lived access token.

**Unix / Git Bash:**

```bash
SHINRAI_MD_CLIENTS=dev:s3cret:alice SHINRAI_MD_SYNTH=1 cargo run -p shinrai-md-gateway

ACCESS=$(curl -s -X POST http://127.0.0.1:8080/v1/auth/token \
  -H 'content-type: application/json' \
  -d '{"grant_type":"client_credentials","client_id":"dev","client_secret":"s3cret"}' \
  | sed -n 's/.*"access_token":"\([^"]*\)".*/\1/p')

websocat "ws://127.0.0.1:8080/v1/ws?token=$ACCESS"
```

**Quick static token** (non-expiring; fine for local smoke tests only):

```bash
SHINRAI_MD_TOKENS=dev:alice SHINRAI_MD_SYNTH=1 cargo run -p shinrai-md-gateway
websocat "ws://127.0.0.1:8080/v1/ws?token=dev"
```

**PowerShell (static token):**

```powershell
$env:SHINRAI_MD_TOKENS = "dev:alice"
$env:SHINRAI_MD_SYNTH = "1"
cargo run -p shinrai-md-gateway
```

**cmd.exe (static token):**

```bat
set SHINRAI_MD_TOKENS=dev:alice
set SHINRAI_MD_SYNTH=1
cargo run -p shinrai-md-gateway
```

You should see `shinrai-md-gateway listening on 127.0.0.1:8080`.

Prefer `Authorization: Bearer` when you can:

```bash
websocat -H "Authorization: Bearer $ACCESS" ws://127.0.0.1:8080/v1/ws
```

Then send:

```json
{"type":"subscribe","symbol":"BTC-USD"}
```

You should get `{"type":"subscribed","instrument_id":3}` and then `tick` frames (`price_scaled` / `qty_lots` integers). Client heartbeats:

```json
{"type":"heartbeat"}
```

Unsubscribe:

```json
{"type":"unsubscribe","symbol":"BTC-USD"}
```

Release binary:

```bash
SHINRAI_MD_TOKENS=dev:alice SHINRAI_MD_SYNTH=1 cargo run -p shinrai-md-gateway --release
```

## Changing the demo / testing another way

| Goal | How |
|---|---|
| Different host/port | `SHINRAI_MD_BIND=0.0.0.0:9090` (or `127.0.0.1:9090`). Update the WebSocket URL. |
| Several users | `SHINRAI_MD_CLIENTS=dev:s3cret:alice,other:pass:bob` or static `SHINRAI_MD_TOKENS=dev:alice,other:bob`. |
| No ticks, only session frames | Omit both `SHINRAI_MD_SYNTH` and `SHINRAI_MD_COINBASE`. After subscribe you still get `subscribed` and periodic server `heartbeat`. |
| Live Coinbase ticks | `SHINRAI_MD_COINBASE=1` (public feed; CI does not enable this). Optional smoke: `cargo test -p shinrai-md-gateway --test coinbase_live -- --ignored`. |
| Fail-closed | Unset both `SHINRAI_MD_TOKENS` and `SHINRAI_MD_CLIENTS`. Connect / token issue returns **401**. |
| Auth rotation | `cargo test -p shinrai-md-gateway --test auth`. |
| Automated WS path | `cargo test -p shinrai-md-gateway --test health` (health, 401, subscribe `BTC-USD`). |
| Other symbols | Fixtures resolve `BTC-USD` / `BTCUSD`, `AAPL`, `ESZ5`. Synth only **publishes** BTC-USD. Subscribing to `AAPL` succeeds but you will not see synth ticks. |
| Invalid token | Connect with `?token=nope` → 401 `invalid_token`. |
| Queue / TTL | Defaults in fanout: queue **64** (drop oldest market-data), max **16** subs, heartbeat every **15** unix seconds, idle TTL **45** seconds. Access token TTL default **60**s (`SHINRAI_MD_ACCESS_TTL`). |
| Historical REST | `cargo test -p shinrai-md-gateway --test historical`. Domain pagination: `cargo test -p shinrai-market-data --lib historical`. |
| Vendor decode only | `cargo test -p shinrai-md-protocol`. Swap fixtures under `crates/protocols/market-data/tests/fixtures/` if you are extending the Coinbase adapter. |
| Paper invariants | `cargo test -p shinrai-paper --test proptest_invariants`. |
| Paper orders over HTTP | `cargo test -p shinrai-order-gateway --test orders`. |
| Portfolio / audit / reconcile | `cargo test -p shinrai-order-gateway --test portfolio`. |
| Live marks (OG → MD quotes) | `cargo test -p shinrai-order-gateway --test live_marks`. Spawns MD gateway on a local port; order gateway fetches `GET /v1/quotes` over HTTP. |
| Durable store (Postgres) | `docker compose up -d postgres` then `SHINRAI_DATABASE_URL=… cargo test -p shinrai-store` / `dual_write`. |

Do not log tokens. Do not commit real secrets. Prefer `SHINRAI_MD_CLIENTS` + short-lived access tokens; `SHINRAI_MD_TOKENS` is a non-expiring bootstrap for local smoke tests only.

## License

MIT (`Cargo.toml`).
