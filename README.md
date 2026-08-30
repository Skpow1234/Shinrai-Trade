# Shinrai-Trade

[![CI](https://github.com/Skpow1234/Shinrai-Trade/actions/workflows/ci.yml/badge.svg)](https://github.com/Skpow1234/Shinrai-Trade/actions/workflows/ci.yml)
[![Rust](https://img.shields.io/badge/rust-1.90.0-orange.svg)](https://www.rust-lang.org/)
[![Tokio](https://img.shields.io/badge/tokio-async-informational.svg)](https://tokio.rs/)
[![Axum](https://img.shields.io/badge/axum-0.8-blue.svg)](https://github.com/tokio-rs/axum)
[![Proptest](https://img.shields.io/badge/proptest-invariants-yellow.svg)](https://github.com/proptest-rs/proptest)
[![License: MIT](https://img.shields.io/badge/license-MIT-green.svg)](https://opensource.org/licenses/MIT)

A Rust trading platform workspace: domain correctness first, then a read-only market-data path. It is **not** an exchange, does **not** hold customer funds, and does **not** accept live orders yet.

The intended product is a trading application connected to a licensed broker or venue. Phase 1 proved money, instruments, an order state machine, a double-entry ledger, and a paper loop **in-process**. Phase 2 is a read-only market-data gateway (normalize, gap/degrade, bounded WebSocket fanout).

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
├── crates/domain/           # money, instruments, ledger, orders, market-data, paper
├── crates/protocols/       # Coinbase adapter + client fanout (no sockets in fanout)
├── crates/services/        # market-data-gateway (Axum binary)
├── crates/testing/         # exchange simulator
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
| `shinrai-md-protocol` | Coinbase Exchange decode, raw journal, feed supervisor |
| `shinrai-md-fanout` | Sessions, authn, bounded queues, heartbeats |
| `shinrai-md-gateway` | `GET /health`, `GET /v1/ws` |
| `shinrai-exchange-simulator` | Scripted venue for paper tests |

## Prerequisites

- [Rustup](https://rustup.rs/). Opening the repo installs **1.90.0** plus `clippy` and `rustfmt` from `rust-toolchain.toml`.
- Optional WebSocket clients for the demo: [`websocat`](https://github.com/vi/websocat) (`cargo install websocat`) or `npx wscat`.

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
cargo build -p shinrai-md-gateway --release
```

The release binary is `target/release/shinrai-md-gateway` (`.exe` on Windows).

## Run

The only process binary today is the market-data gateway. With **no** env vars it binds `127.0.0.1:8080` and **rejects every WebSocket** (fail-closed: empty token table).

```bash
cargo run -p shinrai-md-gateway
```

Health (no auth):

```bash
curl http://127.0.0.1:8080/health
```

Expect `{"status":"ok"}`.

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
cargo test -p shinrai-md-gateway --test health
cargo test -p shinrai-md-fanout --lib overflow_drops_oldest
cargo test -p shinrai-paper --test proptest_invariants
```

CI does **not** open a live Coinbase socket. Vendor tests use recorded JSON under `crates/protocols/market-data/tests/fixtures/`.

## Local demo

Synthetic BTC-USD ticks, one static token, default bind.

**Unix / Git Bash:**

```bash
SHINRAI_MD_TOKENS=dev:alice SHINRAI_MD_SYNTH=1 cargo run -p shinrai-md-gateway
```

**PowerShell:**

```powershell
$env:SHINRAI_MD_TOKENS = "dev:alice"
$env:SHINRAI_MD_SYNTH = "1"
cargo run -p shinrai-md-gateway
```

**cmd.exe:**

```bat
set SHINRAI_MD_TOKENS=dev:alice
set SHINRAI_MD_SYNTH=1
cargo run -p shinrai-md-gateway
```

You should see `shinrai-md-gateway listening on 127.0.0.1:8080`.

Connect (query token is for local tests; prefer `Authorization: Bearer` when you can):

```bash
websocat "ws://127.0.0.1:8080/v1/ws?token=dev"
```

or:

```bash
npx wscat -c "ws://127.0.0.1:8080/v1/ws?token=dev"
```

Bearer instead of query:

```bash
websocat -H "Authorization: Bearer dev" ws://127.0.0.1:8080/v1/ws
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
| Several users | `SHINRAI_MD_TOKENS=dev:alice,other:bob` — first `:` in each pair splits token and subject. |
| No ticks, only session frames | Omit `SHINRAI_MD_SYNTH` (or set it to anything other than `1` / `true` / `yes`). After subscribe you still get `subscribed` and periodic server `heartbeat` (includes `dropped` if the outbound queue overflowed). |
| Fail-closed | Unset `SHINRAI_MD_TOKENS`. WebSocket connect returns **401** `{"type":"error","code":"missing_token"}` or `invalid_token`. |
| Other symbols | Fixtures resolve `BTC-USD` / `BTCUSD`, `AAPL`, `ESZ5`. Synth only **publishes** BTC-USD. Subscribing to `AAPL` succeeds but you will not see synth ticks. |
| Invalid token | Connect with `?token=nope` → 401 `invalid_token`. |
| Queue / TTL | Defaults in fanout: queue **64** (drop oldest market-data), max **16** subs, heartbeat every **15** unix seconds, idle TTL **45** seconds. Change `FanoutConfig` in code or tests (`FanoutConfig::new(...)`) — not env yet. |
| Automated WS path | `cargo test -p shinrai-md-gateway --test health` (health, 401, subscribe `BTC-USD`). |
| Vendor decode only | `cargo test -p shinrai-md-protocol`. Swap fixtures under `crates/protocols/market-data/tests/fixtures/` if you are extending the Coinbase adapter. |
| Paper invariants | `cargo test -p shinrai-paper --test proptest_invariants`. |

Do not log tokens. Do not commit real secrets. `SHINRAI_MD_TOKENS` is a static table for Phase 2; short-lived access tokens and refresh rotation are not implemented yet.

## License

MIT (`Cargo.toml`).
