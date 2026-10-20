# Configuration

All knobs are environment variables. Copy `.env.example` to `.env` for local tools that read dotenv. Never commit `.env` or real secrets.

## Database and messaging

| Variable | Meaning |
|---|---|
| `SHINRAI_DATABASE_URL` | Postgres URL; without it the order gateway is in-memory only |
| `SHINRAI_DATABASE_PASSWORD_FILE` | Docker: path to password file (default `/run/secrets/postgres_password`); builds URL when `SHINRAI_DATABASE_URL` unset |
| `SHINRAI_DATABASE_HOST` / `_USER` / `_NAME` / `_PORT` | Used with password file to build the URL (Compose defaults: `postgres` / `shinrai` / `shinrai` / `5432`) |
| `SHINRAI_OG_OPS_TOKEN_FILE` | Docker: path to ops bearer file (default `/run/secrets/ops_token`) when `SHINRAI_OG_OPS_TOKEN` unset |
| `SHINRAI_DB_POOL_SIZE` | Connection pool size (default 5) |
| `SHINRAI_RUN_MIGRATIONS` | When `1`/`true`, order gateway runs migrations on start (dev) |
| `SHINRAI_OUTBOX_POLL_MS` | Outbox publisher poll interval when Postgres is enabled (default `1000`) |
| `SHINRAI_NATS_URL` | Optional NATS URL for outbox delivery; unset keeps the log sink |
| `SHINRAI_NATS_SUBJECT_PREFIX` | NATS subject prefix (default `shinrai`) |

## Market-data gateway

| Env | Meaning |
|---|---|
| `SHINRAI_MD_BIND` | Listen address (default `127.0.0.1:8080`) |
| `SHINRAI_MD_CLIENTS` | `client_id:secret:subject,...` — issue via `POST /v1/auth/token` |
| `SHINRAI_MD_TOKENS` | `token:subject,...` — non-expiring bootstrap access tokens (local demos) |
| `SHINRAI_MD_ACCESS_TTL` | Access lifetime in seconds (default `60`) |
| `SHINRAI_MD_REFRESH_TTL` | Refresh lifetime in seconds (default `3600`) |
| `SHINRAI_MD_PRODUCTS` | Comma-separated tickers (default `AAPL,ESZ5,BTC-USD`) for synth / Alpaca / futures |
| `SHINRAI_MD_SYNTH` | `1` / `true` — synthetic publisher for configured products |
| `SHINRAI_MD_COINBASE` | `1` / `true` — live Coinbase public WS (BTC-USD). Wins over other feeds |
| `SHINRAI_MD_ALPACA` | `1` / `true` — live Alpaca stock MD (equities); needs keys |
| `SHINRAI_MD_ALPACA_KEY` / `_SECRET` | Alpaca data API credentials (falls back to `SHINRAI_OG_ALPACA_*`) |
| `SHINRAI_MD_ALPACA_FEED` | `iex` (default, free) or `sip` (paid entitlement) |
| `SHINRAI_MD_FUTURES` | `1` / `true` — CME Globex futures via Databento-shaped feed (`ESZ5`); paper if no live WS/key |
| `SHINRAI_MD_FUTURES_KEY` | Licensed futures API key (falls back to `DATABENTO_API_KEY`) |
| `SHINRAI_MD_FUTURES_WS` / `_REST` | Optional live WebSocket / REST origins (omit → paper publisher) |
| `SHINRAI_MD_FUTURES_DATASET` | Dataset id (default `GLBX.MDP3`) |

Prefer `SHINRAI_MD_CLIENTS` + short-lived access tokens. `SHINRAI_MD_TOKENS` is a non-expiring bootstrap for local smoke tests only. Do not log tokens.

## Order gateway — auth and accounts

| Env | Meaning |
|---|---|
| `SHINRAI_OG_BIND` | Listen address (default `127.0.0.1:8081`) |
| `SHINRAI_OG_CLIENTS` | `client_id:secret:subject,...` |
| `SHINRAI_OG_TOKENS` | `token:subject,...` — bootstrap static tokens |
| `SHINRAI_OG_ACCOUNTS` | `subject:account_id,...` — maps auth subject to ledger account |
| `SHINRAI_OG_DEPOSITS` | `account_id:usd_major,...` — bootstrap paper cash |
| `SHINRAI_OG_ACCESS_TTL` / `SHINRAI_OG_REFRESH_TTL` | Token lifetimes (same semantics as MD gateway) |
| `SHINRAI_OG_DEV` | `1` — same empty-field defaults as `--dev` |

`--dev` / `SHINRAI_OG_DEV=1` fills empty auth/account/deposit/mark settings only; any `SHINRAI_OG_*` you set still wins. Defaults: token `dev` → subject `trader` → account `1` with $10,000 paper cash, AAPL mark at `10000`, client `dev`/`s3cret`.

## Order gateway — marks, venue, ops

| Env | Meaning |
|---|---|
| `SHINRAI_OG_MARKS` | Bootstrap marks `SYMBOL:price_scaled,...` |
| `SHINRAI_OG_MD_URL` | MD gateway base URL for `use_live_marks=1` on portfolio |
| `SHINRAI_OG_MD_TOKEN` | Access token when calling the MD gateway |
| `SHINRAI_OG_MD_CLIENT_CERT` / `_KEY` / `_CA` | Optional mTLS PEM paths for OG → MD quote client |
| `SHINRAI_OG_STUCK_AGE_SECS` | Pending OMS age before “stuck” (default `5`) |
| `SHINRAI_OG_VENUE` | `sim` (default), `sandbox`, `rest` / `http`, `licensed`, `fix` / `fix42`, or `alpaca` |
| `SHINRAI_OG_FIX_SENDER_COMP_ID` | FIX tag 49 (default `SHINRAI`) when `venue=fix` |
| `SHINRAI_OG_FIX_TARGET_COMP_ID` | FIX tag 56 (default `SIM`) when `venue=fix` |
| `SHINRAI_OG_FIX_HEARTBEAT_SECS` | Logical heartbeat interval ticks (default `5`) |
| `SHINRAI_OG_FIX_AUTO_FILL` | Auto-fill after New ack (default `true`; use `false` with external TCP) |
| `SHINRAI_OG_FIX_TCP` | `1` — spawn local TCP paper acceptor + initiator session |
| `SHINRAI_OG_FIX_TCP_ADDR` | Loopback bind/connect addr (default `127.0.0.1:19878`) |
| `SHINRAI_OG_FIX_TCP_CONNECT` | Connect initiator to external acceptor `host:port` (no local spawn) |
| `SHINRAI_OG_ALPACA_KEY` / `_SECRET` / `_BASE_URL` | Alpaca paper credentials when `VENUE=alpaca` |
| `SHINRAI_OG_REST_URL` | When `VENUE=rest`, remote paper/broker base URL (local JSON venue if unset) |
| `SHINRAI_OG_REST_TOKEN` | Optional bearer for remote REST venue |
| `SHINRAI_OG_OPS_TOKEN` | Bearer for `/v1/ops*` and `/v1/metrics` (or `?ops_token=`). **Required** outside `--dev` / `SHINRAI_OG_OPS_OPEN` |
| `SHINRAI_OG_OPS_OPEN` | `1` — allow unauthenticated ops when no ops token (dangerous; for demos only) |
| `SHINRAI_OG_DEV_ALLOW_PUBLIC` | `1` — permit `--dev` when bind is not loopback |
| `SHINRAI_OG_OPS_ALLOWLIST` | Comma-separated IPs/CIDRs (or `*`) for ops/metrics; empty = no IP filter |
| `SHINRAI_OG_STORE_FAIL_HARD` | Fail-hard on Postgres write-through (default on; `0`/`false` = fail-soft) |
| `SHINRAI_OG_HA` | `1` — active-passive OMS: Postgres leader lease; non-leaders return `503 not_oms_leader` for `/v1/*` (requires DB) |
| `SHINRAI_OG_INSTANCE_ID` | Stable instance id for the lease / outbox claim (default: `HOSTNAME` / `COMPUTERNAME` / random) |
| `SHINRAI_OG_HA_LEASE_SECS` | Leader lease TTL seconds (default `15`, minimum `3`) |

Venues that need no API key: `sim`, `sandbox`, `rest` without `SHINRAI_OG_REST_URL`, `licensed`, `fix` (in-process or local TCP), and `alpaca` without Alpaca credentials (local mock). Live Alpaca paper needs free keys from Alpaca.

## Order gateway — funds, KYC, audit, risk

| Env | Meaning |
|---|---|
| `SHINRAI_OG_FUNDS_ENABLED` | Paper deposit/withdraw APIs (default on; `0`/`false` disables) |
| `SHINRAI_OG_WITHDRAWALS_ENABLED` | Paper withdrawals (default on) |
| `SHINRAI_OG_WITHDRAW_APPROVAL_THRESHOLD_MINOR` | Large paper withdraw needs `X-Admin-Override` |
| `SHINRAI_OG_KYC_STATUS` | `subject:approved|pending|rejected,...`; empty = all approved |
| `SHINRAI_OG_KYC_VENDOR_URL` / `_TOKEN` / `_TIMEOUT_MS` / `_CACHE_SECS` | Optional HTTP KYC vendor (fail-closed + TTL cache) |
| `SHINRAI_OG_AUDIT_EXPORT_HMAC_KEY` | HMAC-SHA256 key for `GET /v1/ops/audit/export` signature |
| `SHINRAI_OG_AUDIT_EXPORT_RETENTION_SECS` | Only export audit rows newer than this window |
| `SHINRAI_OG_RESTRICTED_SYMBOLS` | Comma-separated symbols restricted at bootstrap |
| `SHINRAI_OG_ADMIN_OVERRIDE_TOKEN` | Bearer for `X-Admin-Override` on restricted instruments |
| `SHINRAI_RISK_COLLAR_BPS` | Price collar vs mark (`0` = off; when `>0`, missing/zero mark fails closed) |
| `SHINRAI_RISK_MAX_DAILY_LOSS_MINOR` | Day loss limit in quote minor units (0 = off) |
| `SHINRAI_RISK_MAX_ASSET_CLASS_NOTIONAL_MINOR` | Cap on same-asset-class exposure + new order (0 = off) |
| `SHINRAI_RISK_MARKET_SESSION` | Optional `open_secs:close_secs` from midnight UTC |
| `SHINRAI_RISK_ALLOW_SHORT` / `SHINRAI_RISK_MAX_SHORT_LOTS` | Short policy overlays on demo limits |

## Telemetry

| Env | Meaning |
|---|---|
| `SHINRAI_LOG` | `tracing` filter (falls back to `RUST_LOG`, default `info`) |
| `SHINRAI_OTEL_ENDPOINT` | OTLP/HTTP base URL (e.g. `http://127.0.0.1:4318`); also accepts `OTEL_EXPORTER_OTLP_ENDPOINT` |

Order-path spans (`order.submit`, `order.cancel`, `order.replace`) carry `account_id`, `client_order_id` / `order_id`, and `outcome`. HTTP requests get a `tower-http` trace layer. Without an OTLP endpoint, spans still appear on stderr via the fmt layer.
