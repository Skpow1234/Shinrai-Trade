# Local demo

## Paper trader UI (recommended for orders)

1. `cargo run -p shinrai-order-gateway -- --dev` (or `./scripts/dev-ui.sh` / `.\scripts\dev-ui.ps1`).
2. Open `http://127.0.0.1:8081/ui` — on localhost the page auto-signs in with token `dev`.
3. Place a limit order (qty in lots, price in scaled ticks — e.g. `10000` = $100.00).

The UI stores the access token in **`sessionStorage`** and sends `Authorization: Bearer …`. Ops remains at `/v1/ops` (separate ops token when `SHINRAI_OG_OPS_TOKEN` is set).

Manual env (same defaults as `--dev`):

```bash
SHINRAI_OG_TOKENS=dev:trader \
SHINRAI_OG_ACCOUNTS=trader:1 \
SHINRAI_OG_DEPOSITS=1:10000 \
cargo run -p shinrai-order-gateway
```

## Equity / futures market data

Multi-product **synth** (no API keys):

```bash
SHINRAI_MD_SYNTH=1 \
SHINRAI_MD_PRODUCTS=AAPL,ESZ5,BTC-USD \
SHINRAI_MD_TOKENS=dev:alice \
cargo run -p shinrai-md-gateway
```

Subscribe on the WebSocket to `AAPL` / `ESZ5` / `BTC-USD` (see below).

**Alpaca** licensed stock stream (equities). Futures in `PRODUCTS` get a synth sidecar:

```bash
SHINRAI_MD_ALPACA=1 \
SHINRAI_MD_ALPACA_KEY=… \
SHINRAI_MD_ALPACA_SECRET=… \
SHINRAI_MD_PRODUCTS=AAPL,ESZ5 \
SHINRAI_MD_TOKENS=dev:alice \
cargo run -p shinrai-md-gateway
```

## FIX over TCP (paper)

Default `SHINRAI_OG_VENUE=fix` stays **in-process**. To exercise a real TCP session against the bundled paper acceptor:

```bash
SHINRAI_OG_VENUE=fix \
SHINRAI_OG_FIX_TCP=1 \
SHINRAI_OG_TOKENS=dev:trader \
SHINRAI_OG_ACCOUNTS=trader:1 \
SHINRAI_OG_DEPOSITS=1:10000 \
cargo run -p shinrai-order-gateway
```

Or point the initiator at an external FIX 4.2 acceptor (Logon / Heartbeat / D / F / G / ER subset):

```bash
SHINRAI_OG_VENUE=fix \
SHINRAI_OG_FIX_TCP_CONNECT=127.0.0.1:9876 \
SHINRAI_OG_FIX_AUTO_FILL=false \
cargo run -p shinrai-order-gateway -- --dev
```

## Market-data WebSocket

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

Prefer `Authorization: Bearer` when you can:

```bash
websocat -H "Authorization: Bearer $ACCESS" ws://127.0.0.1:8080/v1/ws
```

**Quick static token** (non-expiring; local smoke tests only):

```bash
SHINRAI_MD_TOKENS=dev:alice SHINRAI_MD_SYNTH=1 cargo run -p shinrai-md-gateway
websocat "ws://127.0.0.1:8080/v1/ws?token=dev"
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

Then send:

```json
{"type":"subscribe","symbol":"BTC-USD"}
```

You should get `{"type":"subscribed","instrument_id":3}` and then `tick` frames (`price_scaled` / `qty_lots` integers).

Heartbeat:

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
| Other symbols | Fixtures resolve `BTC-USD` / `BTCUSD`, `AAPL`, `ESZ5`. Synth publishes all of `SHINRAI_MD_PRODUCTS` (default includes AAPL/ESZ5/BTC-USD). Alpaca live covers equities only. |
| Invalid token | Connect with `?token=nope` → 401 `invalid_token`. |
| Queue / TTL | Defaults in fanout: queue **64** (drop oldest market-data), max **16** subs, heartbeat every **15** unix seconds, idle TTL **45** seconds. Access token TTL default **60**s (`SHINRAI_MD_ACCESS_TTL`). |
| Historical REST | `cargo test -p shinrai-md-gateway --test historical`. Domain pagination: `cargo test -p shinrai-market-data --lib historical`. |
| Vendor decode only | `cargo test -p shinrai-md-protocol`. Swap fixtures under `crates/protocols/market-data/tests/fixtures/` if extending Coinbase. |
| Paper invariants | `cargo test -p shinrai-paper --test proptest_invariants`. |
| Paper orders over HTTP | `cargo test -p shinrai-order-gateway --test orders`. |
| Portfolio / audit / reconcile | `cargo test -p shinrai-order-gateway --test portfolio`. |
| Live marks (OG → MD quotes) | `cargo test -p shinrai-order-gateway --test live_marks`. |
| Durable store (Postgres) | `docker compose up -d postgres` then `SHINRAI_DATABASE_URL=… cargo test -p shinrai-store` / `dual_write`. |

Do not log tokens. Do not commit real secrets.
