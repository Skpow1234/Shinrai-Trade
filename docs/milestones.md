# Milestones

Delivery is tracked as **phases** in the README history and crate docs. There is no separate milestones tracker in-repo (private notes in `docs/design.md` / `docs/objectives-expanded.md` are gitignored). Status below reflects what the codebase and docs claim today.

## Phase summary

| Phase | Goal | Status |
|---|---|---|
| **1** | In-process money, instruments, OMS FSM, double-entry ledger, paper loop | Done |
| **2** | Read-only market-data gateway: normalize, gap/degrade, bounded WS fanout, historical bars/trades, short-lived auth | Done |
| **3** | Paper order HTTP gateway: pre-trade risk → OMS → venue → ledger | Done |
| **3.5+** | Postgres write-through, hydrate on restart, transactional outbox (+ optional NATS) | Done |
| **4** | Licensed-broker paper path (Alpaca), FIX 4.2 subset (in-process), KYC/approvals/audit export | Done (with gaps) |
| **5** | Paper deposit/withdraw/balances (ledger only, `mode: "paper"`) | Done (paper only) |

## Phase 1 — Domain core

- Integer money and instrument master (`shinrai-money`, `shinrai-instruments`)
- Order FSM + idempotent store (`shinrai-orders`)
- Double-entry ledger (`shinrai-ledger`)
- Paper loop with simulated venue (`shinrai-paper`, `shinrai-exchange-simulator`)
- Proptest invariants on the paper loop

## Phase 2 — Market data

- Coinbase public adapter + feed supervisor (`shinrai-md-protocol`)
- Fanout hub: auth, heartbeats, bounded queues (`shinrai-md-fanout`)
- Axum gateway: auth, bars/trades/quotes, WebSocket (`shinrai-md-gateway`)
- Synth publisher and seeded historical archive for demos

## Phase 3 — Paper order gateway

- Authenticated HTTP OMS (`shinrai-order-gateway`)
- Pre-trade risk fail-closed (`shinrai-risk`)
- Portfolio, audit, reconciliation APIs
- Embedded paper trader UI at `/ui`

## Phase 3.5+ — Durability

- `shinrai-store` migrations (orders, ledger, audit chain, outbox, approvals)
- Write-through persist before ack; hydrate on restart
- Outbox with `SKIP LOCKED`; log sink or NATS (`compose.yaml`)

## Phase 4 — Broker paper path

Complete for the licensed-broker **paper** path:

- Alpaca paper REST (`SHINRAI_OG_VENUE=alpaca`) with async fill polling
- Durable OMS + drop-copy + session cursor
- KYC / approvals / audit export
- Alpaca-sourced EOD position recon
- Local FIX 4.2 subset (`SHINRAI_OG_VENUE=fix`) — SOH-delimited wire over an **in-process** acceptor

### Known gaps (Phase 4+)

- FIX: **no vendor TCP** yet (loopback / in-process only)
- Live market data: Coinbase public BTC-USD only; equities/futures on the master are not live-streamed from a licensed feed
- Single-process OMS; no documented multi-instance leader election beyond outbox claim

## Phase 5 — Paper funds

- `GET /v1/accounts/balances`, `POST …/deposit`, `POST …/withdraw`
- Double-entry paper ledger entries with idempotency keys
- Responses always `mode: "paper"`

Real custody / bank rails stay gated on regulation and licensed partners. This phase is explicitly **not** customer-money custody.

## Recently completed (packaging / ops)

- Multi-stage `Dockerfile` for both gateways
- `compose.yaml` `--profile full` runs Postgres, NATS, MD + order gateways
- Non-dev ops fail-closed without `SHINRAI_OG_OPS_TOKEN` (`ops_auth_required`)
- Refuse `--dev` on non-loopback bind unless `SHINRAI_OG_DEV_ALLOW_PUBLIC=1`

## Suggested next (priority)

1. TLS termination example (Caddy) + compose secrets for DB password
2. FIX vendor TCP session to a real counterparty
3. Licensed equity/futures market-data feeds
4. Async Alpaca / non-blocking venue HTTP on the Tokio runtime
5. Multi-instance OMS / HA story if running more than one order-gateway process
6. Persist risk kills/limits across hydrate; collar fail-closed without marks

## Full backlog (500+)

See **[milestones-backlog.md](milestones-backlog.md)** — 800 code-derived improvement milestones (`M-0001`…) across every crate (correctness, security, venues, deploy, UX, polish).
