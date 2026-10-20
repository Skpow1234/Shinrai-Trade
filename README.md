# Shinrai-Trade

[![CI](https://github.com/Skpow1234/Shinrai-Trade/actions/workflows/ci.yml/badge.svg)](https://github.com/Skpow1234/Shinrai-Trade/actions/workflows/ci.yml)
[![Rust](https://img.shields.io/badge/rust-1.90.0-orange.svg)](https://www.rust-lang.org/)
[![Tokio](https://img.shields.io/badge/tokio-async-informational.svg)](https://tokio.rs/)
[![Axum](https://img.shields.io/badge/axum-0.8-blue.svg)](https://github.com/tokio-rs/axum)
[![Proptest](https://img.shields.io/badge/proptest-invariants-yellow.svg)](https://github.com/proptest-rs/proptest)
[![License: MIT](https://img.shields.io/badge/license-MIT-green.svg)](https://opensource.org/licenses/MIT)

A Rust trading platform workspace: domain correctness first, then a read-only market-data path, then authenticated paper orders. It is **not** an exchange and does **not** hold customer funds in production.

The intended product is a trading application connected to a licensed broker or venue. Phases 1–5 cover in-process domain, market-data gateway, paper OMS, durable Postgres, broker paper venues (Alpaca / FIX subset), and paper funds — see [milestones](docs/milestones.md).

## Quick start

```bash
# Toolchain: Rust 1.90.0 from rust-toolchain.toml (+ clippy, rustfmt)
cargo build --workspace --all-features

# Paper trader UI (token: dev)
cargo run -p shinrai-order-gateway -- --dev
# → http://127.0.0.1:8081/ui

# Optional durable store
docker compose up -d postgres
cp .env.example .env
```

Scripts: `./scripts/dev-ui.sh` · `.\scripts\dev-ui.ps1` · `scripts\dev-ui.cmd`

## Documentation

| Doc | Contents |
|---|---|
| [docs/](docs/README.md) | Documentation index |
| [Architecture](docs/architecture.md) | Tech stack and crate layout |
| [Getting started](docs/getting-started.md) | Prerequisites, Postgres/NATS, build, run |
| [Configuration](docs/configuration.md) | Environment variables |
| [Endpoints](docs/endpoints.md) | HTTP / WebSocket API |
| [Local demo](docs/demo.md) | Smoke tests and demo knobs |
| [Testing](docs/testing.md) | Test and CI commands |
| [Milestones](docs/milestones.md) | Phases 1–5 and known gaps |
| [Deployment](docs/deployment.md) | Local run and deploy options |

## Layout

```text
Shinrai-Trade/
├── crates/domain/           # money, instruments, ledger, orders, paper, risk, …
├── crates/protocols/       # MD vendor + fanout, execution venues
├── crates/services/        # shinrai-md-gateway, shinrai-order-gateway
├── crates/infrastructure/  # store, messaging, telemetry
├── crates/testing/         # exchange simulator
├── docs/                   # operational docs (this tree)
├── compose.yaml            # Postgres + optional NATS
└── .env.example
```

## License

MIT (`Cargo.toml`).
