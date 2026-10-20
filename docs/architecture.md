# Architecture

A Rust trading platform workspace: domain correctness first, then a read-only market-data path, then authenticated paper orders. It is **not** an exchange and does **not** hold customer funds in production.

The intended product is a trading application connected to a licensed broker or venue. Domain crates stay independent of Axum, databases, and a specific venue. Gateways are thin I/O edges.

## Tech

| Area | Choice |
|---|---|
| Language / edition | Rust 1.90.0 (`rust-toolchain.toml`), edition 2021, workspace resolver 2 |
| Async / HTTP | Tokio, Axum 0.8 (HTTP + WebSocket) |
| Serialization | Serde / `serde_json` (JSON at the client and vendor edges only) |
| Money / prices | Integer minor units, ticks, and lots — **no `f32`/`f64` on money or prices** |
| Persistence | PostgreSQL via sqlx (`shinrai-store`) |
| Messaging | Optional NATS for outbox (`shinrai-messaging`) |
| Telemetry | `tracing` + optional OTLP (`shinrai-telemetry`) |
| Tests | Colocated unit tests, crate `tests/`, Proptest on the paper loop |
| CI | GitHub Actions: `fmt`, Clippy `-D warnings`, tests on Ubuntu / Windows / macOS, release build |

## Layout

```text
Shinrai-Trade/
├── crates/domain/           # money, instruments, ledger, orders, market-data, paper, risk
├── crates/protocols/       # Coinbase adapter + client fanout (no sockets in fanout)
├── crates/services/        # market-data-gateway, order-gateway (Axum binaries)
├── crates/infrastructure/  # shinrai-store (Postgres), messaging, telemetry
├── crates/testing/         # exchange simulator
├── compose.yaml            # optional Postgres + NATS for durable store / event bus
├── .env.example            # SHINRAI_* placeholders
└── .github/workflows/ci.yml
```

## Crates

| Crate | Role |
|---|---|
| `shinrai-money` | Currencies and integer money |
| `shinrai-instruments` | Instrument master, tick/lot grid, aliases (`AAPL`, `ESZ5`, `BTC-USD`) |
| `shinrai-ledger` | Double-entry journal |
| `shinrai-orders` | Order FSM + idempotent store |
| `shinrai-market-data` | Tick journal, OHLCV, L2 book, replay |
| `shinrai-paper` | Paper loop: reserve → venue → fill → settle |
| `shinrai-risk` | Pre-trade checks: buying power, limits, kill switches |
| `shinrai-audit` | Append-only audit trail for the trading path |
| `shinrai-portfolio` | Cash, positions, mark-to-market P&L snapshots |
| `shinrai-md-protocol` | Coinbase Exchange decode, raw journal, feed supervisor |
| `shinrai-md-fanout` | Sessions, authn, bounded queues, heartbeats |
| `shinrai-md-gateway` | Health, bars/trades/quotes, WebSocket |
| `shinrai-order-gateway` | Orders, portfolio, audit, funds, reconciliation, ops, UI |
| `shinrai-execution` | `ExecutionVenue` trait; sandbox, REST, Alpaca, FIX subset |
| `shinrai-store` | PostgreSQL: orders, ledger, audit, transactional outbox |
| `shinrai-messaging` | Outbox sinks: log (default) + optional NATS publish |
| `shinrai-telemetry` | Process tracing + optional OTLP export |
| `shinrai-exchange-simulator` | Scripted venue for paper tests |

## Request path (order gateway)

Auth → rate limit → pre-trade risk → `PaperEngine` (OMS + ledger) → `ExecutionVenue` → optional Postgres write-through → audit / outbox.
