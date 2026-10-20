# Milestones

Delivery is tracked as **phases** in the README history and crate docs. There is no separate milestones tracker in-repo (private notes in `docs/design.md` / `docs/objectives-expanded.md` are gitignored). Status below reflects what the codebase and docs claim today.

## Phase summary

| Phase | Goal | Status |
|---|---|---|
| **1** | In-process money, instruments, OMS FSM, double-entry ledger, paper loop | Done |
| **2** | Read-only market-data gateway: normalize, gap/degrade, bounded WS fanout, historical bars/trades, short-lived auth | Done |
| **3** | Paper order HTTP gateway: pre-trade risk → OMS → venue → ledger | Done |
| **3.5+** | Postgres write-through, hydrate on restart, transactional outbox (+ optional NATS) | Done |
| **4** | Licensed-broker paper path (Alpaca), FIX 4.2 subset (in-process + TCP), KYC/approvals/audit export | Done (with gaps) |
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
- Local FIX 4.2 subset (`SHINRAI_OG_VENUE=fix`) — SOH-delimited wire in-process **or TCP** (`SHINRAI_OG_FIX_TCP` / `_CONNECT`)
- FIX session recovery: inbound gap → `ResendRequest` (`35=2`); retransmit with `PossDupFlag` (`43=Y`) or `SequenceReset` gap-fill (`35=4` / `123=Y`)

### Known gaps (Phase 4+)

- FIX subset still omits some admin/business edges (e.g. fuller business Reject mapping, Drop Copy fanout to a second CompID)
- Futures MD: Databento-shaped paper/live for `ESZ5` (`SHINRAI_MD_FUTURES`) with mbp-10 L2; raw CME MDP3 multicast not wired
- Multi-instance OMS is **active-passive** (`SHINRAI_OG_HA`); not multi-writer active-active (hydrate-on-promote + follower warm standby)

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
- **Compose file secrets** for Postgres password + ops token (`deploy/secrets/`, `init-secrets`)
- **Caddy TLS overlay** (`deploy/Caddyfile`, `compose.tls.yaml`, profile `tls`)
- **FIX TCP**: initiator + loopback paper acceptor (`SHINRAI_OG_FIX_TCP`) or external connect (`SHINRAI_OG_FIX_TCP_CONNECT`)
- **FIX session recovery**: ResendRequest / PossDup retransmit / SequenceReset gap-fill (initiator + paper acceptor)
- **FIX admin / drop-copy**: Reject `35=3`, TestRequest `35=1` (+ Heartbeat TestReqID echo), `SHINRAI_OG_FIX_SESSION=drop_copy`
- **Equity/futures MD**: Alpaca stock stream (`SHINRAI_MD_ALPACA`) + Databento GLBX futures (`SHINRAI_MD_FUTURES`, paper or licensed, mbp-10 L2) + multi-product synth
- **Databento L2**: mbp-10 snapshot/delta decode + paper book publisher (`SHINRAI_MD_FUTURES_SCHEMA`); raw MDP3 multicast still external
- **Async venue HTTP**: Alpaca + remote REST use async `reqwest` with `block_in_place` on the multi-thread Tokio runtime (no `reqwest::blocking`)
- **Async KYC HTTP**: compliance vendor uses async `reqwest` + `block_in_place` (no `reqwest::blocking`)
- **Multi-instance OMS HA**: Postgres `oms_leader_lease` + `SHINRAI_OG_HA` (active-passive; followers `503 not_oms_leader`; outbox publishes only on leader; hydrate-on-promote + follower warm rehydrate)
- **Durable risk control plane**: `risk_state` table (kills/limits/restrictions) restored on hydrate; collar fail-closed when `collar_bps>0` and no mark
- **Risk ops audit**: `risk_ops_audit` + `GET /v1/ops/risk-audit` for every ops risk mutation (and system persist-fail kills / approval overrides)
- **KYC fail-closed**: empty `SHINRAI_OG_KYC_STATUS` blocks trading outside `--dev` / `SHINRAI_OG_KYC_OPEN` (`M-0346`)

## Suggested next (priority)

1. FIX ExecutionReport Reject mapping (`M-0125`)
2. Raw CME MDP3 multicast (licensed binary UDP) — optional when vendor JSON feed unavailable
3. Ops audit trail polish / readiness probes

## Full backlog (500+)

See **[milestones-backlog.md](milestones-backlog.md)** — 800 code-derived improvement milestones (`M-0001`…) across every crate (correctness, security, venues, deploy, UX, polish).
