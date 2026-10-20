# Endpoints

Defaults: market-data gateway `127.0.0.1:8080`, order gateway `127.0.0.1:8081`.

Auth for client APIs: `Authorization: Bearer <token>` or `?token=`. Prefer the header.

Ops routes (`/v1/ops*`, `/v1/metrics`): require `SHINRAI_OG_OPS_TOKEN` (header or `?ops_token=`) unless `--dev` / `SHINRAI_OG_DEV` / `SHINRAI_OG_OPS_OPEN=1`. Without a token in non-dev mode the response is **401** `ops_auth_required`.

---

## Market-data gateway (`shinrai-md-gateway`)

| Method | Path | Auth | Description |
|---|---|---|---|
| `GET` | `/health` | No | Liveness; includes feed mode |
| `POST` | `/v1/auth/token` | Client credentials / refresh | Issue or refresh access tokens |
| `POST` | `/v1/auth/revoke` | Body token | Revoke access or refresh |
| `GET` | `/v1/bars` | Yes | Historical OHLCV bars |
| `GET` | `/v1/trades` | Yes | Trade prints (cursor pagination) |
| `GET` | `/v1/quotes` | Yes | Latest quote for a symbol |
| `GET` | `/v1/ws` | Yes | WebSocket fanout |

### Health

```bash
curl http://127.0.0.1:8080/health
# {"status":"ok","service":"market-data-gateway","feed":"none|synth|alpaca|coinbase"}
```

### Auth

Short-lived access tokens (default **60s**) and rotating refresh tokens.

```bash
# Issue
curl -s -X POST http://127.0.0.1:8080/v1/auth/token \
  -H 'content-type: application/json' \
  -d '{"grant_type":"client_credentials","client_id":"dev","client_secret":"s3cret"}'

# Refresh (old refresh becomes invalid; reuse revokes the token family)
curl -s -X POST http://127.0.0.1:8080/v1/auth/token \
  -H 'content-type: application/json' \
  -d '{"grant_type":"refresh_token","refresh_token":"<refresh>"}'

# Revoke
curl -s -X POST http://127.0.0.1:8080/v1/auth/revoke \
  -H 'content-type: application/json' \
  -d '{"token":"<access_or_refresh>"}'
```

### Historical REST

```bash
# Minute bars (also: 1s / 1h / 1d, or raw seconds e.g. interval=60)
curl "http://127.0.0.1:8080/v1/bars?symbol=BTC-USD&interval=1m&limit=10&token=$ACCESS"

# Trade prints; pass next_cursor from the response for the next page
curl "http://127.0.0.1:8080/v1/trades?symbol=BTC-USD&limit=50&token=$ACCESS"
```

Optional filters: `start` / `end` (logical or Unix seconds). Prices and sizes are scaled integers (`*_scaled`, `*_lots`). Startup seeds ~120 synthetic BTC-USD trades so these work without a publisher.

### WebSocket

See [Local demo](demo.md) for subscribe / heartbeat / unsubscribe frames.

---

## Order gateway (`shinrai-order-gateway`)

When `SHINRAI_DATABASE_URL` is set, Postgres is **required write-through**: submit/cancel ack only after a successful transactional persist. Persist failure returns **503** `persist_failed` and engages the global kill switch.

| Method | Path | Auth | Description |
|---|---|---|---|
| `GET` | `/health` | No | Liveness |
| `GET` | `/ui` | No (UI uses token) | Paper trader HTML |
| `POST` | `/v1/auth/token` | Client credentials / refresh | Issue or refresh |
| `POST` | `/v1/auth/revoke` | Body token | Revoke |
| `GET` | `/v1/orders` | Yes | List orders |
| `POST` | `/v1/orders` | Yes | Submit order |
| `GET` | `/v1/orders/{id}` | Yes | Get order |
| `POST` | `/v1/orders/{id}/cancel` | Yes | Cancel |
| `POST` | `/v1/orders/{id}/replace` | Yes | Replace working limit |
| `GET` | `/v1/portfolio` | Yes | Cash, positions, P&L |
| `GET` | `/v1/audit` | Yes | Append-only audit trail |
| `GET` | `/v1/accounts/balances` | Yes | Paper balances |
| `POST` | `/v1/accounts/deposit` | Yes | Paper deposit |
| `POST` | `/v1/accounts/withdraw` | Yes | Paper withdraw |
| `GET` | `/v1/reconciliation` | Yes | OMS vs venue snapshot |
| `GET` | `/v1/metrics` | Ops | Counters / stuck / health |
| `GET` | `/v1/ops` | Ops | Ops HTML dashboard |
| `GET` | `/v1/ops/stuck-orders` | Ops | Stuck pending OMS |
| `GET`/`POST` | `/v1/ops/risk` | Ops | Risk control plane |
| `POST` | `/v1/ops/reconciliation/eod` | Ops | EOD broker recon |
| `GET`/`POST` | `/v1/ops/approvals` | Ops | Approvals |
| `GET` | `/v1/ops/withdraw-audit` | Ops | Paper withdraw audit |
| `GET` | `/v1/ops/audit/export` | Ops | Signed audit export |

### Orders

```bash
curl -s -X POST "http://127.0.0.1:8081/v1/orders?token=dev" \
  -H 'content-type: application/json' \
  -d '{"client_order_id":"o1","symbol":"AAPL","side":"Buy","qty":10,"price":10000}'

# Optional order_type (Limit|Market, default Limit) and tif (GTC|IOC|FOK, default GTC)
# -d '{"client_order_id":"o2","symbol":"AAPL","side":"Buy","qty":10,"price":10000,"order_type":"Limit","tif":"IOC"}'

curl "http://127.0.0.1:8081/v1/orders?token=dev"

curl -s -X POST "http://127.0.0.1:8081/v1/orders/1/replace?token=dev" \
  -H 'content-type: application/json' \
  -d '{"qty":6,"price":9000}'
```

Pre-trade risk runs before the OMS. Insufficient buying power returns **422** with `"code":"insufficient_buying_power"`. Duplicate `client_order_id` for the same account is idempotent.

### Portfolio, audit, reconciliation

```bash
curl "http://127.0.0.1:8081/v1/portfolio?token=dev&use_stored_marks=1"
curl "http://127.0.0.1:8081/v1/portfolio?token=dev&use_live_marks=1"
curl "http://127.0.0.1:8081/v1/audit?token=dev"
curl "http://127.0.0.1:8081/v1/reconciliation?token=dev"
```

### Paper funds (Phase 5 — ledger only)

Responses are always `mode: "paper"`. This is **not** bank rails or customer-money custody.

```bash
curl "http://127.0.0.1:8081/v1/accounts/balances?token=dev"
curl -X POST "http://127.0.0.1:8081/v1/accounts/deposit?token=dev" \
  -H 'content-type: application/json' \
  -d '{"idempotency_key":"dep-1","amount_minor":50000,"currency":"USD"}'
curl -X POST "http://127.0.0.1:8081/v1/accounts/withdraw?token=dev" \
  -H 'content-type: application/json' \
  -d '{"idempotency_key":"wd-1","amount_minor":10000}'
```

### Ops and metrics

Outside `--dev`, set `SHINRAI_OG_OPS_TOKEN` (Compose `full` uses `compose-ops`).

```bash
curl http://127.0.0.1:8081/v1/metrics
curl "http://127.0.0.1:8081/v1/ops/stuck-orders?max_age_secs=5"
curl http://127.0.0.1:8081/v1/ops/risk
curl -X POST http://127.0.0.1:8081/v1/ops/risk \
  -H 'content-type: application/json' \
  -d '{"global_kill":true,"limits":{"collar_bps":100}}'

curl -X POST http://127.0.0.1:8081/v1/ops/reconciliation/eod \
  -H 'content-type: application/json' \
  -d '{"cash":[{"account_id":1,"currency":"USD","minor_units":1000000}],"positions":[],"fills":[]}'

# When SHINRAI_OG_VENUE=alpaca
curl -X POST http://127.0.0.1:8081/v1/ops/reconciliation/eod \
  -H 'content-type: application/json' \
  -d '{"source":"alpaca","account_id":1}'

curl -X POST http://127.0.0.1:8081/v1/ops/approvals \
  -H 'content-type: application/json' \
  -d '{"account_id":1,"symbol":"AAPL"}'

curl "http://127.0.0.1:8081/v1/ops/withdraw-audit"
```

### UI

- Paper trader: `http://127.0.0.1:8081/ui` (on localhost auto-signs in with token `dev` when `--dev` defaults apply)
- Ops dashboard: `http://127.0.0.1:8081/v1/ops`
