# Milestones backlog

Actionable improvement milestones mined from the codebase (Phases 1–5 complete for paper scope).
Items marked **(done)** were implemented in the packaging/ops-hardening pass.

Total: **800** milestones.

> This is a living engineering backlog, not a committed roadmap. Prioritize with product needs.

## How to use

1. Pick a theme cluster (security, deploy, venue, risk durability).
2. Open a PR that closes one or a few IDs (`M-0123`).
3. When implementing, update status inline or move IDs into Completed.

## Completed recently

- **M-deploy-docker**: Multi-stage `Dockerfile` for `md-gateway` / `order-gateway`
- **M-deploy-compose-full**: `compose.yaml` `--profile full` runs Postgres, NATS, both gateways
- **M-ops-fail-closed**: Non-dev order gateway requires `SHINRAI_OG_OPS_TOKEN` (`ops_auth_required`)
- **M-dev-bind-guard**: Refuse `--dev` on non-loopback bind unless `SHINRAI_OG_DEV_ALLOW_PUBLIC=1`
- **M-deploy-compose-secrets**: File secrets for Postgres password + ops token (`deploy/secrets/`, init scripts)
- **M-deploy-caddy-tls**: Caddy TLS termination (`deploy/Caddyfile`, `compose.tls.yaml`, profile `tls`)
- **M-fix-tcp**: FIX TCP initiator + loopback paper acceptor / external `TCP_CONNECT`
- **M-md-alpaca-equity**: Alpaca stock MD vendor + gateway feed; multi-product synth for AAPL/ESZ5/BTC-USD

## Backlog

### api-product

- **M-0001**: Webhooks for fill/cancel events
- **M-0002**: GraphQL optional facade
- **M-0003**: Batch order submit API
- **M-0004**: Account hierarchy (master/sub)
- **M-0005**: API key scopes (trade/read/ops)
- **M-0006**: OAuth2/OIDC instead of static clients
- **M-0007**: Pagination cursors everywhere
- **M-0008**: Conditional requests (ETag) for portfolio
- **M-0009**: HTTP/2 and gRPC dual stack study
- **M-0010**: Client SDKs (Rust/TS/Python)
- **M-0011**: Sandbox vs production base URL docs
- **M-0012**: Idempotent funds across retries documented
- **M-0013**: Versioned /v2 API plan
- **M-0014**: Deprecation policy
- **M-0015**: Public status page hooks

### audit

- **M-0016**: Verify hash chain on hydrate; reject breaks
- **M-0017**: Stop accepting from_parts_legacy in prod hydrate
- **M-0018**: ~~Audit kinds for ops/KYC/approvals/risk changes~~ **partial** (risk via `risk_ops_audit`; KYC/approvals still separate tables)
- **M-0019**: Require correlation id on all order-path records
- **M-0020**: HMAC export key rotation + dual-key verify
- **M-0021**: Retention/compaction for audit_records
- **M-0022**: Metrics: chain break counters
- **M-0023**: Structured detail JSON instead of free-text
- **M-0024**: Audit search by client_order_id
- **M-0025**: Tamper-evident export packaging (zip+sig)
- **M-0026**: PII redaction policy for audit detail
- **M-0027**: Clock skew tolerance documentation
- **M-0028**: Audit mirror to object storage
- **M-0029**: Unit test: broken chain fails hydrate
- **M-0030**: Benchmark append under lock contention

### compliance-product

- **M-0031**: Real custody rails (partner-gated)
- **M-0032**: KYC vendor production adapters
- **M-0033**: AML transaction monitoring stub
- **M-0034**: Tax lot reporting export
- **M-0035**: Regulatory audit export formats
- **M-0036**: Data residency documentation
- **M-0037**: Customer PII store separation
- **M-0038**: Right-to-erasure vs audit retention policy
- **M-0039**: Suitability questionnaire stub
- **M-0040**: Pattern day trader flag stub
- **M-0041**: Statement generation (PDF) stub
- **M-0042**: Trade confirmation email stub
- **M-0043**: Market data redistribution license tracking
- **M-0044**: Broker agreement version pinning
- **M-0045**: Disaster recovery RPO/RTO targets

### data-quality

- **M-0046**: Reconcile marks vs last trade
- **M-0047**: Stale quote detector job
- **M-0048**: Duplicate trade print filter
- **M-0049**: Clock discipline NTP check
- **M-0050**: Instrument tick size change handling
- **M-0051**: Corporate action file ingest
- **M-0052**: Reference data freshness metric
- **M-0053**: Bad tick spike filter
- **M-0054**: Cross-venue symbol normalization
- **M-0055**: Holiday calendar data pack

### deploy

- **M-0056**: Dockerfile for gateways (done)
- **M-0057**: Compose profile full for gateways (done)
- **M-0058**: Example Caddyfile TLS termination
- **M-0059**: Compose secrets for DB password
- **M-0060**: ~~Multi-instance OMS leader election~~ **done** (`oms_leader_lease` + `SHINRAI_OG_HA`)
- **M-0060b**: ~~Hydrate-on-promote / follower warm standby~~ **done** (rehydrate before `leader:true`; warm rehydrate while follower)
- **M-0061**: Document single-writer sticky sessions
- **M-0062**: Readiness vs liveness probes
- **M-0063**: Postgres backup/restore runbook
- **M-0064**: Horizontal MD fanout scaling design
- **M-0065**: systemd unit examples
- **M-0066**: Nightly ignored live tests workflow
- **M-0067**: SBOM / dependency audit job
- **M-0068**: Release artifact upload (versioned binaries)
- **M-0069**: Image scan in CI (trivy/grype)
- **M-0070**: Pin base image digests
- **M-0071**: Non-root already in Dockerfile — verify
- **M-0072**: Resource limits in compose
- **M-0073**: docker compose production overlay example
- **M-0074**: Helm chart scaffold
- **M-0075**: Terraform module stub for VM+RDS
- **M-0076**: .env.production.example without weak secrets
- **M-0077**: Document SHINRAI_OG_OPS_OPEN danger
- **M-0078**: CHANGELOG automation
- **M-0079**: Semver release process
- **M-0080**: Supply-chain provenance (cosign)

### docs-dx

- **M-0081**: Keep milestones-backlog synced with code
- **M-0082**: Generate endpoints from route macros
- **M-0083**: Architecture diagram for hydrate cursor gap
- **M-0084**: ADR: single-mutex OMS
- **M-0085**: Remove dead allow(dead_code) in compliance
- **M-0086**: Consistent error enums paper/execution/store
- **M-0087**: Document workspace feature flags
- **M-0088**: CONTRIBUTING.md
- **M-0089**: CODEOWNERS
- **M-0090**: Public API surface review per lib.rs
- **M-0091**: Optional clippy nursery CI lane
- **M-0092**: Example postman/bruno collection
- **M-0093**: Decision log for venue selection
- **M-0094**: Glossary (lots, ticks, logical time)
- **M-0095**: Onboarding: 15-minute first trade path

### exchange-simulator

- **M-0096**: More fault modes (disconnect mid-fill, reorder)
- **M-0097**: Wire md_skip_seq into gateway tests
- **M-0098**: Remove allow(dead_code) fields
- **M-0099**: Scripted scenario files
- **M-0100**: Parallel sim instances multi-account chaos
- **M-0101**: IOC/FOK fidelity tests
- **M-0102**: Latency injection distribution
- **M-0103**: Partial fill sequences from YAML
- **M-0104**: Sim exchange admin HTTP for tests
- **M-0105**: Deterministic RNG seed from test name

### execution

- **M-0106**: FIX vendor TCP session
- **M-0107**: ~~FIX ResendRequest / gap fill / seq reset~~ **done** (initiator gap → `35=2`; PossDup `43=Y` retransmit; `SequenceReset` `35=4`/`123=Y` gap-fill; paper acceptor store)
- **M-0108**: FIX checksum / body-length hardening
- **M-0109**: Alpaca: replace eprintln with metrics
- **M-0110**: ~~Alpaca: async HTTP (no blocking reqwest)~~ **done** (`RemoteAlpacaHttp` + `block_on_venue`)
- **M-0111**: Alpaca: cancel/replace race edge cases
- **M-0112**: Alpaca: secrets redaction audit
- **M-0113**: REST remote: TLS pinning / mTLS
- **M-0114**: REST: timeout / retry / circuit breaker
- **M-0115**: Licensed sandbox: real broker session codes
- **M-0116**: Sandbox: latency/jitter policies
- **M-0117**: Unify inflight hydrate APIs across venues
- **M-0118**: ExecutionVenue async trait / non-blocking poll (HTTP path done via `block_on_venue`; full async trait still open)
- **M-0119**: Nightly CI for ignored live venue tests
- **M-0120**: Remove allow(dead_code) in alpaca
- **M-0121**: EOD fetch failure fail-closed for recon
- **M-0122**: Broker API rate-limit awareness
- **M-0123**: ~~FIX Drop Copy session type~~ **done** (`FixSessionKind::DropCopy` / `SHINRAI_OG_FIX_SESSION=drop_copy`)
- **M-0123b**: ~~FIX Reject `35=3` + TestRequest `35=1`~~ **done** (initiator + paper acceptor)
- **M-0124**: FIX Order Cancel Request full path
- **M-0125**: ~~FIX ExecutionReport Reject mapping~~ **done** (ER `150=8` + `OrdRejReason`/`Text`; CancelReject as `35=9` + `CxlRejReason`)
- **M-0126**: Venue capability matrix documentation
- **M-0127**: Paper→live kill switch per venue
- **M-0128**: Venue health probe endpoint
- **M-0129**: Mock Alpaca clock for determinism
- **M-0130**: Contract tests for ExecutionReport shape
- **M-0131**: Interactive Brokers adapter stub
- **M-0132**: CME iLink stub milestone
- **M-0133**: Venue session journal encryption at rest
- **M-0134**: Replace venue without restart (hot swap)
- **M-0135**: Chaos: partial fill then disconnect

### hardening

- **M-0136**: Fail closed on unknown VenueKind env values
- **M-0137**: Validate all SHINRAI_* numeric env at boot
- **M-0138**: Reject empty client secrets
- **M-0139**: Max body size middleware on Axum
- **M-0140**: Request timeout middleware
- **M-0141**: Slowloris protection notes
- **M-0142**: Disable TRACE/TRACK methods
- **M-0143**: Security headers (HSTS via proxy)
- **M-0144**: Content-Type enforcement on POST
- **M-0145**: JSON depth limit

### instruments

- **M-0146**: Persist InstrumentMaster beyond in-memory fixtures
- **M-0147**: Hot-reload instrument master without restart
- **M-0148**: Expand phase1_master for live equities/futures symbols
- **M-0149**: Harden ISIN check-digit coverage
- **M-0150**: Centralize tick/lot grid enforcement on OMS submit
- **M-0151**: Corporate-action / symbol rename alias history
- **M-0152**: Validate AssetClass on every instrument insert
- **M-0153**: Concurrent read benchmarks for large masters
- **M-0154**: Document display vs wire symbol conventions
- **M-0155**: Dead-code / unused fixture cleanup
- **M-0156**: Support CFI codes on instruments
- **M-0157**: MIC / venue instrument mapping table
- **M-0158**: Options series instrument type
- **M-0159**: Instrument status: trading halt flag
- **M-0160**: Export instrument master as JSON snapshot API
- **M-0161**: Import instrument CSV for ops bootstrap
- **M-0162**: Unit tests for alias collision rejection
- **M-0163**: Fuzzy symbol search for UI autocomplete
- **M-0164**: Instrument versioning for audit forensics
- **M-0165**: Multi-currency quote currency per instrument

### ledger

- **M-0166**: Persist journal with snapshots (not memory-only)
- **M-0167**: Incremental trial-balance for large journals
- **M-0168**: Multi-currency cash accounts beyond USD paper
- **M-0169**: Document chart of accounts (fees/PnL/clearing)
- **M-0170**: Harden reverse-entry under concurrent posts
- **M-0171**: Archive/bound journal growth
- **M-0172**: Fail closed on empty or single-sided postings
- **M-0173**: Proptest: Applied posts preserve trial balance
- **M-0174**: Detect position memo vs cash mix-ups
- **M-0175**: Export journal without cloning full history
- **M-0176**: Ledger account freeze flag
- **M-0177**: Sub-ledger per strategy tag
- **M-0178**: Accrual vs cash accounting mode flag
- **M-0179**: Fee amortization helpers
- **M-0180**: Nightly ledger integrity job
- **M-0181**: SQL CHECK that debits equal credits per entry
- **M-0182**: Ledger pagination API
- **M-0183**: Reverse-by-id audit linkage
- **M-0184**: Currency revaluation journal entry type
- **M-0185**: Load test 1M journal lines

### market-data-domain

- **M-0186**: Persist MdJournal / historical bars
- **M-0187**: Gap-recovery policy beyond clear L2 book
- **M-0188**: Multi-instrument book sharding
- **M-0189**: Bar aggregator late/out-of-order policy
- **M-0190**: Replay determinism across platforms
- **M-0191**: Checksum algorithm versioning
- **M-0192**: Synth feed: spreads and halts
- **M-0193**: Consumer lag/watermark metrics hooks
- **M-0194**: Historical retention limits
- **M-0195**: Proptest OHLCV under shuffled ticks
- **M-0196**: Document ts_logical vs wall-clock
- **M-0197**: L2 book snapshot REST export
- **M-0198**: Trade condition codes
- **M-0199**: Auction / imbalance event kind
- **M-0200**: Session markers (open/close)
- **M-0201**: Consolidate ticks to 1s bars online
- **M-0202**: Historical query max-range guard
- **M-0203**: Book checksum on every snapshot
- **M-0204**: MD domain fuzz targets
- **M-0205**: Benchmark bar roll under high TPS

### md-fanout

- **M-0206**: Env-tunable FanoutConfig
- **M-0207**: Backpressure signal on drop (not silent)
- **M-0208**: Per-subject subscription ACLs
- **M-0209**: Hash stored tokens (no plaintext map)
- **M-0210**: Refresh-token rotation reuse detection
- **M-0211**: Minimize secrets lifetime in Session
- **M-0212**: Shard hub by instrument
- **M-0213**: Prometheus-style drop/queue metrics
- **M-0214**: Protocol versioning / schema negotiation
- **M-0215**: Fuzz client commands
- **M-0216**: Max message size enforcement
- **M-0217**: Per-IP connection limits
- **M-0218**: Subscribe rate limit per session
- **M-0219**: Binary frame option (vs JSON only)
- **M-0220**: Session dump for support tooling
- **M-0221**: Integration test: 1k concurrent subscribers
- **M-0222**: Idle TTL vs access TTL alignment tests
- **M-0223**: Document fanout drop policy in endpoints.md
- **M-0224**: Optional durable last-value cache per symbol
- **M-0225**: Graceful hub shutdown drains queues

### md-gateway

- **M-0226**: Replace eprintln with tracing
- **M-0227**: Auth fail-closed when no tokens outside demo
- **M-0228**: Health distinguishes feed stale vs up
- **M-0229**: Historical endpoint DoS input limits
- **M-0230**: WebSocket idle timeout aligned with hub TTL
- **M-0231**: Metrics endpoint for drops/reconnects
- **M-0232**: Configurable Coinbase product list
- **M-0233**: Scheduled coinbase_live workflow
- **M-0234**: TLS/mTLS story for MD gateway
- **M-0235**: Backpressure when history mutex contended
- **M-0236**: Seed journal retention policy
- **M-0237**: Document bind address security defaults
- **M-0238**: Readiness probe separate from liveness
- **M-0239**: Graceful shutdown of WS sessions
- **M-0240**: OpenAPI for MD REST
- **M-0241**: CORS policy for browser clients
- **M-0242**: Rate limit auth/token endpoint
- **M-0243**: Structured access log (no tokens)
- **M-0244**: Multi-instance sticky fanout design
- **M-0245**: Integration test: auth revoke mid-stream

### md-protocol

- **M-0246**: ~~Second vendor adapter (Alpaca/IEX/CME)~~ **done** (Alpaca equities + Databento GLBX futures paper/live)
- **M-0246b**: ~~Databento mbp-10 L2 schemas~~ **done** (snapshot/delta decode + paper book; `SHINRAI_MD_FUTURES_SCHEMA`)
- **M-0247**: Equity/futures live products beyond BTC-USD
- **M-0248**: Authenticated vendor streams
- **M-0249**: Supervisor multi-product watch lists
- **M-0250**: Configurable stale SLA via env
- **M-0251**: Typed degrade events from vendor errors
- **M-0252**: Recorded multi-frame integration fixtures
- **M-0253**: Raw journal durability for compliance
- **M-0254**: Gap detection metrics export
- **M-0255**: Expand fixtures/*.json corpus
- **M-0256**: Vendor adapter conformance test harness
- **M-0257**: Reconnect backoff jitter config
- **M-0258**: Snapshot REST rate-limit handling
- **M-0259**: Symbol map vendor→instrument_id table
- **M-0260**: Nightly coinbase_live in scheduled CI
- **M-0261**: Document FeedSupervisor state machine
- **M-0262**: Handle vendor trading halt messages
- **M-0263**: Decode incremental L2 for Coinbase
- **M-0264**: Vendor clock vs logical clock sync
- **M-0265**: Circuit breaker on vendor error rate

### messaging

- **M-0266**: NATS JetStream durable consumers
- **M-0267**: Exactly-once story with inbox consumers
- **M-0268**: Subject schema registry / versioning
- **M-0269**: Dead-letter topic for poison payloads
- **M-0270**: NATS auth (token/nkey)
- **M-0271**: Payload PII redaction checker
- **M-0272**: Reconnect/backoff metrics
- **M-0273**: LogSink truncate large payloads
- **M-0274**: CloudEvents envelope option
- **M-0275**: Outbox→webhook sink

### money

- **M-0276**: Property tests for Money overflow on checked_add/mul
- **M-0277**: Reject exotic Unicode digits in parse_major
- **M-0278**: Document JPY scale-0 and high-scale crypto edge cases
- **M-0279**: Add Money::checked_div and fee allocation helpers
- **M-0280**: Fail closed on currency mismatch at more call sites
- **M-0281**: Introduce MinorUnits newtype across crates
- **M-0282**: Fuzz parse_major in CI
- **M-0283**: Audit #[must_use] on arithmetic helpers
- **M-0284**: Benchmark Money hot paths under load
- **M-0285**: Serialize Money with explicit scale in JSON APIs
- **M-0286**: Add Money::abs and saturating helpers with tests
- **M-0287**: Reject negative deposits at Money construction sites
- **M-0288**: Cross-crate Money Display format consistency
- **M-0289**: Document rounding policy for major↔minor conversion
- **M-0290**: Invariant: scale never exceeds currency definition

### ops-runbooks

- **M-0291**: Runbook: clear global kill
- **M-0292**: Runbook: stuck PendingNew
- **M-0293**: Runbook: persist_degraded recovery
- **M-0294**: Runbook: broken audit chain
- **M-0295**: Runbook: outbox backlog
- **M-0296**: Runbook: Postgres failover
- **M-0297**: Runbook: rotate ops token
- **M-0298**: Runbook: rotate DB password
- **M-0299**: Runbook: venue disconnect
- **M-0300**: Runbook: hydrate failure on boot

### order-gateway-core

- **M-0301**: Document persist-degraded recovery path
- **M-0302**: Gate store_fail_hard=false to non-prod
- **M-0303**: No silent REST venue fallback to local
- **M-0304**: Withdraw audit persist must not only eprintln
- **M-0305**: Poisoned mutex: fail closed vs into_inner
- **M-0306**: Alpaca remote build: Result not panic
- **M-0307**: Shard or remove global Mutex<PaperEngine>
- **M-0308**: Incremental persist (not full ledger each time)
- **M-0309**: Bound rate limiter HashMap growth
- **M-0310**: Tune demo rate limit for abuse protection
- **M-0311**: ~~Avoid blocking venue HTTP on async runtime~~ **done** (async reqwest + `block_in_place` / dedicated thread)
- **M-0312**: Reduce engine clones on read paths
- **M-0313**: ~~Hydrate risk kills/limits/KYC fully~~ **done** (kills/limits/restrictions via `risk_state`; KYC still env/vendor)
- **M-0314**: Durable venue cursor design after hydrate
- **M-0315**: Configurable outbox stale reclaim window
- **M-0316**: Outbox poison queue metrics
- **M-0317**: Chunk large dual-write batches
- **M-0318**: Approvals/withdraw audit hydrate completeness
- **M-0319**: Expand chaos tests beyond persist degrade
- **M-0320**: Market orders: price optional in HTTP
- **M-0321**: Idempotency-Key header for submit
- **M-0322**: Paginate list orders
- **M-0323**: Correlation id always on audit+outbox
- **M-0324**: Funds amount scale validation UX
- **M-0325**: Consistent problem+json error schema
- **M-0326**: OpenAPI generation for OG
- **M-0327**: Replace vs in-flight cancel concurrency
- **M-0328**: Document X-Admin-Override security
- **M-0329**: Startup banner: ops auth open/closed
- **M-0330**: Readiness: hydrate complete && !persist_degraded

### order-gateway-observability

- **M-0331**: Richer GatewayMetrics (histograms)
- **M-0332**: OTLP metrics instruments
- **M-0333**: Structured JSON logs option
- **M-0334**: Persist/outbox/recon SLO gauges
- **M-0335**: Trace venue round-trips
- **M-0336**: Request-id middleware
- **M-0337**: Prometheus scrape endpoint
- **M-0338**: Grafana dashboard JSON
- **M-0339**: Alert rules for kill switch / stuck orders
- **M-0340**: Log sampling for high-volume ticks path

### order-gateway-security

- **M-0341**: Require ops token outside --dev (done: ops_allow_open)
- **M-0342**: Prefer header-only ops auth (deprecate query token)
- **M-0343**: Trusted-proxy config for X-Forwarded-For
- **M-0344**: IPv6 CIDR support in ops allowlist
- **M-0345**: Fail-closed allowlist when token set
- **M-0346**: ~~KYC empty map fail-closed in production~~ **done** (empty map → pending unless `SHINRAI_OG_DEV` / `SHINRAI_OG_KYC_OPEN`)
- **M-0347**: ~~Non-blocking KYC HTTP (no reqwest::blocking)~~ **done** (async `reqwest` + `block_on_http`)
- **M-0348**: Dual-control log for admin overrides
- **M-0349**: Discourage static SHINRAI_OG_TOKENS in prod mode
- **M-0350**: Refuse --dev on non-loopback (done)
- **M-0351**: Constant-time ops bearer compare
- **M-0352**: Secrets redaction audit for Debug impls
- **M-0353**: Rotate admin override token runbook
- **M-0354**: CSP headers on /ui and /v1/ops HTML
- **M-0355**: SameSite cookie option if session cookies added
- **M-0356**: Security.txt / SECURITY.md
- **M-0357**: Dependency audit CI (cargo deny/audit)
- **M-0358**: SBOM generation on release
- **M-0359**: Pen-test checklist in docs
- **M-0360**: Threat model document

### order-gateway-ux

- **M-0361**: Paper UI light theme / a11y
- **M-0362**: UI: replace / TIF / market order controls
- **M-0363**: Ops HTML CSP hardening
- **M-0364**: UI: do not default token=dev when non-local
- **M-0365**: Live marks UX when MD down
- **M-0366**: UI footer: Phase 5 not custody
- **M-0367**: UI link to ops respects ops auth
- **M-0368**: Mobile-responsive paper UI
- **M-0369**: Keyboard shortcuts for submit
- **M-0370**: Order blotter filters
- **M-0371**: Export fills CSV from UI
- **M-0372**: Toast errors with stable codes
- **M-0373**: i18n stub
- **M-0374**: Dark/light system preference
- **M-0375**: UI e2e smoke (playwright) in CI

### orders

- **M-0376**: Validate FSM reachability on Order::restore
- **M-0377**: Persist pending_replace_* across hydrate
- **M-0378**: Exhaustive transition matrix tests status×event
- **M-0379**: Cap seen_execs growth on long-lived orders
- **M-0380**: Reject duplicate client_order_id when payload differs
- **M-0381**: Event-sourced rebuild API from event_log
- **M-0382**: Support DAY and GTD TIF
- **M-0383**: Add stop / stop-limit / trailing order types
- **M-0384**: Stable reject reason codes (not free-form only)
- **M-0385**: Proptest: cum_qty + leaves_qty == order_qty
- **M-0386**: Shardable OrderStore for multi-engine
- **M-0387**: Idempotency key length/charset enforcement
- **M-0388**: Order amend history query API
- **M-0389**: Mass cancel by account/symbol
- **M-0390**: Pegged order type stub
- **M-0391**: Iceberg / display qty fields
- **M-0392**: Order tags / client metadata map
- **M-0393**: Time-in-force expire worker
- **M-0394**: Reject zero-qty replaces explicitly
- **M-0395**: Serialize OrderStatus as stable wire enum
- **M-0396**: Order age metrics histogram
- **M-0397**: Cancel-reject reason taxonomy
- **M-0398**: Working order book view by symbol
- **M-0399**: Stress test 100k orders in store
- **M-0400**: Document FSM diagram in docs/

### paper

- **M-0401**: Heal/fail-closed when hydrate reserve underfunds
- **M-0402**: Durable venue session cursor reapply design
- **M-0403**: Reduce PaperEngine clone cost under lock
- **M-0404**: Async venue poll without holding global mutex
- **M-0405**: Replace flow tests across all venues
- **M-0406**: Reconcile auto-remediation actions
- **M-0407**: Persist broker EOD injection ownership
- **M-0408**: Mark staleness fail-closed for collar (missing mark done; staleness TTL still open)
- **M-0409**: Document logical_now vs wall clock for stuck
- **M-0410**: Expand proptest_invariants scenarios
- **M-0411**: Unify VenueHandle error mapping
- **M-0412**: Drain reports atomically with persist watermark
- **M-0413**: Detect orphan venue fills after reconnect
- **M-0414**: Maker/taker fee model in settlement
- **M-0415**: Partial-fill reserve release rounding invariants
- **M-0416**: Paper engine feature flag matrix in CI
- **M-0417**: Simulate exchange downtime chaos test
- **M-0418**: Multi-account concurrent submit stress
- **M-0419**: Snapshot/restore PaperEngine golden files
- **M-0420**: Document paper vs live venue contract
- **M-0421**: Position avg-cost vs FIFO mode
- **M-0422**: Corporate action adjust on paper positions
- **M-0423**: Paper dividend credit stub
- **M-0424**: Borrow interest accrual stub
- **M-0425**: Engine metrics: submit latency histogram

### performance

- **M-0426**: Flamegraph order submit path
- **M-0427**: Reduce serde allocations on hot path
- **M-0428**: Arena/bump for tick fanout
- **M-0429**: io_uring study for FIX TCP
- **M-0430**: Connection pooling for MD→vendor
- **M-0431**: Zero-copy WS frames where safe
- **M-0432**: CPU affinity docs for trading hosts
- **M-0433**: NUMA-aware deployment notes
- **M-0434**: Latency SLO: p99 submit < X ms paper
- **M-0435**: Memory budget per gateway process
- **M-0436**: Avoid HashMap hasher DoS (RandomState ok?)
- **M-0437**: Compress historical REST responses
- **M-0438**: Cache instrument resolve
- **M-0439**: Batch mark updates
- **M-0440**: Outbox publish pipelining

### polish:shinrai-audit

- **M-0441**: shinrai-audit: Add missing # Errors docs on pub fns
- **M-0442**: shinrai-audit: Add examples in crate-level docs
- **M-0443**: shinrai-audit: Review Clippy pedantic locally and fix safe lints
- **M-0444**: shinrai-audit: Increase unit test coverage for private helpers
- **M-0445**: shinrai-audit: Add Debug redaction for secret-bearing types
- **M-0446**: shinrai-audit: Stabilize public re-exports in lib.rs
- **M-0447**: shinrai-audit: Add integration test module outline
- **M-0448**: shinrai-audit: Document feature flags in Cargo.toml
- **M-0449**: shinrai-audit: Add criterion benches for hottest fn
- **M-0450**: shinrai-audit: Remove unused dependencies if any
- **M-0451**: shinrai-audit: Align error Display messages with HTTP codes
- **M-0452**: shinrai-audit: Add tracing events at module boundaries
- **M-0453**: shinrai-audit: Document thread-safety of shared types
- **M-0454**: shinrai-audit: Add quickcheck/proptest where math-heavy
- **M-0455**: shinrai-audit: Review panic! sites; convert to Result

### polish:shinrai-exchange-simulator

- **M-0456**: shinrai-exchange-simulator: Add missing # Errors docs on pub fns
- **M-0457**: shinrai-exchange-simulator: Add examples in crate-level docs
- **M-0458**: shinrai-exchange-simulator: Review Clippy pedantic locally and fix safe lints
- **M-0459**: shinrai-exchange-simulator: Increase unit test coverage for private helpers
- **M-0460**: shinrai-exchange-simulator: Add Debug redaction for secret-bearing types
- **M-0461**: shinrai-exchange-simulator: Stabilize public re-exports in lib.rs
- **M-0462**: shinrai-exchange-simulator: Add integration test module outline
- **M-0463**: shinrai-exchange-simulator: Document feature flags in Cargo.toml
- **M-0464**: shinrai-exchange-simulator: Add criterion benches for hottest fn
- **M-0465**: shinrai-exchange-simulator: Remove unused dependencies if any
- **M-0466**: shinrai-exchange-simulator: Align error Display messages with HTTP codes
- **M-0467**: shinrai-exchange-simulator: Add tracing events at module boundaries
- **M-0468**: shinrai-exchange-simulator: Document thread-safety of shared types
- **M-0469**: shinrai-exchange-simulator: Add quickcheck/proptest where math-heavy
- **M-0470**: shinrai-exchange-simulator: Review panic! sites; convert to Result

### polish:shinrai-execution

- **M-0471**: shinrai-execution: Add missing # Errors docs on pub fns
- **M-0472**: shinrai-execution: Add examples in crate-level docs
- **M-0473**: shinrai-execution: Review Clippy pedantic locally and fix safe lints
- **M-0474**: shinrai-execution: Increase unit test coverage for private helpers
- **M-0475**: shinrai-execution: Add Debug redaction for secret-bearing types
- **M-0476**: shinrai-execution: Stabilize public re-exports in lib.rs
- **M-0477**: shinrai-execution: Add integration test module outline
- **M-0478**: shinrai-execution: Document feature flags in Cargo.toml
- **M-0479**: shinrai-execution: Add criterion benches for hottest fn
- **M-0480**: shinrai-execution: Remove unused dependencies if any
- **M-0481**: shinrai-execution: Align error Display messages with HTTP codes
- **M-0482**: shinrai-execution: Add tracing events at module boundaries
- **M-0483**: shinrai-execution: Document thread-safety of shared types
- **M-0484**: shinrai-execution: Add quickcheck/proptest where math-heavy
- **M-0485**: shinrai-execution: Review panic! sites; convert to Result

### polish:shinrai-instruments

- **M-0486**: shinrai-instruments: Add missing # Errors docs on pub fns
- **M-0487**: shinrai-instruments: Add examples in crate-level docs
- **M-0488**: shinrai-instruments: Review Clippy pedantic locally and fix safe lints
- **M-0489**: shinrai-instruments: Increase unit test coverage for private helpers
- **M-0490**: shinrai-instruments: Add Debug redaction for secret-bearing types
- **M-0491**: shinrai-instruments: Stabilize public re-exports in lib.rs
- **M-0492**: shinrai-instruments: Add integration test module outline
- **M-0493**: shinrai-instruments: Document feature flags in Cargo.toml
- **M-0494**: shinrai-instruments: Add criterion benches for hottest fn
- **M-0495**: shinrai-instruments: Remove unused dependencies if any
- **M-0496**: shinrai-instruments: Align error Display messages with HTTP codes
- **M-0497**: shinrai-instruments: Add tracing events at module boundaries
- **M-0498**: shinrai-instruments: Document thread-safety of shared types
- **M-0499**: shinrai-instruments: Add quickcheck/proptest where math-heavy
- **M-0500**: shinrai-instruments: Review panic! sites; convert to Result

### polish:shinrai-ledger

- **M-0501**: shinrai-ledger: Add missing # Errors docs on pub fns
- **M-0502**: shinrai-ledger: Add examples in crate-level docs
- **M-0503**: shinrai-ledger: Review Clippy pedantic locally and fix safe lints
- **M-0504**: shinrai-ledger: Increase unit test coverage for private helpers
- **M-0505**: shinrai-ledger: Add Debug redaction for secret-bearing types
- **M-0506**: shinrai-ledger: Stabilize public re-exports in lib.rs
- **M-0507**: shinrai-ledger: Add integration test module outline
- **M-0508**: shinrai-ledger: Document feature flags in Cargo.toml
- **M-0509**: shinrai-ledger: Add criterion benches for hottest fn
- **M-0510**: shinrai-ledger: Remove unused dependencies if any
- **M-0511**: shinrai-ledger: Align error Display messages with HTTP codes
- **M-0512**: shinrai-ledger: Add tracing events at module boundaries
- **M-0513**: shinrai-ledger: Document thread-safety of shared types
- **M-0514**: shinrai-ledger: Add quickcheck/proptest where math-heavy
- **M-0515**: shinrai-ledger: Review panic! sites; convert to Result

### polish:shinrai-market-data

- **M-0516**: shinrai-market-data: Add missing # Errors docs on pub fns
- **M-0517**: shinrai-market-data: Add examples in crate-level docs
- **M-0518**: shinrai-market-data: Review Clippy pedantic locally and fix safe lints
- **M-0519**: shinrai-market-data: Increase unit test coverage for private helpers
- **M-0520**: shinrai-market-data: Add Debug redaction for secret-bearing types
- **M-0521**: shinrai-market-data: Stabilize public re-exports in lib.rs
- **M-0522**: shinrai-market-data: Add integration test module outline
- **M-0523**: shinrai-market-data: Document feature flags in Cargo.toml
- **M-0524**: shinrai-market-data: Add criterion benches for hottest fn
- **M-0525**: shinrai-market-data: Remove unused dependencies if any
- **M-0526**: shinrai-market-data: Align error Display messages with HTTP codes
- **M-0527**: shinrai-market-data: Add tracing events at module boundaries
- **M-0528**: shinrai-market-data: Document thread-safety of shared types
- **M-0529**: shinrai-market-data: Add quickcheck/proptest where math-heavy
- **M-0530**: shinrai-market-data: Review panic! sites; convert to Result

### polish:shinrai-md-fanout

- **M-0531**: shinrai-md-fanout: Add missing # Errors docs on pub fns
- **M-0532**: shinrai-md-fanout: Add examples in crate-level docs
- **M-0533**: shinrai-md-fanout: Review Clippy pedantic locally and fix safe lints
- **M-0534**: shinrai-md-fanout: Increase unit test coverage for private helpers
- **M-0535**: shinrai-md-fanout: Add Debug redaction for secret-bearing types
- **M-0536**: shinrai-md-fanout: Stabilize public re-exports in lib.rs
- **M-0537**: shinrai-md-fanout: Add integration test module outline
- **M-0538**: shinrai-md-fanout: Document feature flags in Cargo.toml
- **M-0539**: shinrai-md-fanout: Add criterion benches for hottest fn
- **M-0540**: shinrai-md-fanout: Remove unused dependencies if any
- **M-0541**: shinrai-md-fanout: Align error Display messages with HTTP codes
- **M-0542**: shinrai-md-fanout: Add tracing events at module boundaries
- **M-0543**: shinrai-md-fanout: Document thread-safety of shared types
- **M-0544**: shinrai-md-fanout: Add quickcheck/proptest where math-heavy
- **M-0545**: shinrai-md-fanout: Review panic! sites; convert to Result

### polish:shinrai-md-gateway

- **M-0546**: shinrai-md-gateway: Add missing # Errors docs on pub fns
- **M-0547**: shinrai-md-gateway: Add examples in crate-level docs
- **M-0548**: shinrai-md-gateway: Review Clippy pedantic locally and fix safe lints
- **M-0549**: shinrai-md-gateway: Increase unit test coverage for private helpers
- **M-0550**: shinrai-md-gateway: Add Debug redaction for secret-bearing types
- **M-0551**: shinrai-md-gateway: Stabilize public re-exports in lib.rs
- **M-0552**: shinrai-md-gateway: Add integration test module outline
- **M-0553**: shinrai-md-gateway: Document feature flags in Cargo.toml
- **M-0554**: shinrai-md-gateway: Add criterion benches for hottest fn
- **M-0555**: shinrai-md-gateway: Remove unused dependencies if any
- **M-0556**: shinrai-md-gateway: Align error Display messages with HTTP codes
- **M-0557**: shinrai-md-gateway: Add tracing events at module boundaries
- **M-0558**: shinrai-md-gateway: Document thread-safety of shared types
- **M-0559**: shinrai-md-gateway: Add quickcheck/proptest where math-heavy
- **M-0560**: shinrai-md-gateway: Review panic! sites; convert to Result

### polish:shinrai-md-protocol

- **M-0561**: shinrai-md-protocol: Add missing # Errors docs on pub fns
- **M-0562**: shinrai-md-protocol: Add examples in crate-level docs
- **M-0563**: shinrai-md-protocol: Review Clippy pedantic locally and fix safe lints
- **M-0564**: shinrai-md-protocol: Increase unit test coverage for private helpers
- **M-0565**: shinrai-md-protocol: Add Debug redaction for secret-bearing types
- **M-0566**: shinrai-md-protocol: Stabilize public re-exports in lib.rs
- **M-0567**: shinrai-md-protocol: Add integration test module outline
- **M-0568**: shinrai-md-protocol: Document feature flags in Cargo.toml
- **M-0569**: shinrai-md-protocol: Add criterion benches for hottest fn
- **M-0570**: shinrai-md-protocol: Remove unused dependencies if any
- **M-0571**: shinrai-md-protocol: Align error Display messages with HTTP codes
- **M-0572**: shinrai-md-protocol: Add tracing events at module boundaries
- **M-0573**: shinrai-md-protocol: Document thread-safety of shared types
- **M-0574**: shinrai-md-protocol: Add quickcheck/proptest where math-heavy
- **M-0575**: shinrai-md-protocol: Review panic! sites; convert to Result

### polish:shinrai-messaging

- **M-0576**: shinrai-messaging: Add missing # Errors docs on pub fns
- **M-0577**: shinrai-messaging: Add examples in crate-level docs
- **M-0578**: shinrai-messaging: Review Clippy pedantic locally and fix safe lints
- **M-0579**: shinrai-messaging: Increase unit test coverage for private helpers
- **M-0580**: shinrai-messaging: Add Debug redaction for secret-bearing types
- **M-0581**: shinrai-messaging: Stabilize public re-exports in lib.rs
- **M-0582**: shinrai-messaging: Add integration test module outline
- **M-0583**: shinrai-messaging: Document feature flags in Cargo.toml
- **M-0584**: shinrai-messaging: Add criterion benches for hottest fn
- **M-0585**: shinrai-messaging: Remove unused dependencies if any
- **M-0586**: shinrai-messaging: Align error Display messages with HTTP codes
- **M-0587**: shinrai-messaging: Add tracing events at module boundaries
- **M-0588**: shinrai-messaging: Document thread-safety of shared types
- **M-0589**: shinrai-messaging: Add quickcheck/proptest where math-heavy
- **M-0590**: shinrai-messaging: Review panic! sites; convert to Result

### polish:shinrai-money

- **M-0591**: shinrai-money: Add missing # Errors docs on pub fns
- **M-0592**: shinrai-money: Add examples in crate-level docs
- **M-0593**: shinrai-money: Review Clippy pedantic locally and fix safe lints
- **M-0594**: shinrai-money: Increase unit test coverage for private helpers
- **M-0595**: shinrai-money: Add Debug redaction for secret-bearing types
- **M-0596**: shinrai-money: Stabilize public re-exports in lib.rs
- **M-0597**: shinrai-money: Add integration test module outline
- **M-0598**: shinrai-money: Document feature flags in Cargo.toml
- **M-0599**: shinrai-money: Add criterion benches for hottest fn
- **M-0600**: shinrai-money: Remove unused dependencies if any
- **M-0601**: shinrai-money: Align error Display messages with HTTP codes
- **M-0602**: shinrai-money: Add tracing events at module boundaries
- **M-0603**: shinrai-money: Document thread-safety of shared types
- **M-0604**: shinrai-money: Add quickcheck/proptest where math-heavy
- **M-0605**: shinrai-money: Review panic! sites; convert to Result

### polish:shinrai-order-gateway

- **M-0606**: shinrai-order-gateway: Add missing # Errors docs on pub fns
- **M-0607**: shinrai-order-gateway: Add examples in crate-level docs
- **M-0608**: shinrai-order-gateway: Review Clippy pedantic locally and fix safe lints
- **M-0609**: shinrai-order-gateway: Increase unit test coverage for private helpers
- **M-0610**: shinrai-order-gateway: Add Debug redaction for secret-bearing types
- **M-0611**: shinrai-order-gateway: Stabilize public re-exports in lib.rs
- **M-0612**: shinrai-order-gateway: Add integration test module outline
- **M-0613**: shinrai-order-gateway: Document feature flags in Cargo.toml
- **M-0614**: shinrai-order-gateway: Add criterion benches for hottest fn
- **M-0615**: shinrai-order-gateway: Remove unused dependencies if any
- **M-0616**: shinrai-order-gateway: Align error Display messages with HTTP codes
- **M-0617**: shinrai-order-gateway: Add tracing events at module boundaries
- **M-0618**: shinrai-order-gateway: Document thread-safety of shared types
- **M-0619**: shinrai-order-gateway: Add quickcheck/proptest where math-heavy
- **M-0620**: shinrai-order-gateway: Review panic! sites; convert to Result

### polish:shinrai-orders

- **M-0621**: shinrai-orders: Add missing # Errors docs on pub fns
- **M-0622**: shinrai-orders: Add examples in crate-level docs
- **M-0623**: shinrai-orders: Review Clippy pedantic locally and fix safe lints
- **M-0624**: shinrai-orders: Increase unit test coverage for private helpers
- **M-0625**: shinrai-orders: Add Debug redaction for secret-bearing types
- **M-0626**: shinrai-orders: Stabilize public re-exports in lib.rs
- **M-0627**: shinrai-orders: Add integration test module outline
- **M-0628**: shinrai-orders: Document feature flags in Cargo.toml
- **M-0629**: shinrai-orders: Add criterion benches for hottest fn
- **M-0630**: shinrai-orders: Remove unused dependencies if any
- **M-0631**: shinrai-orders: Align error Display messages with HTTP codes
- **M-0632**: shinrai-orders: Add tracing events at module boundaries
- **M-0633**: shinrai-orders: Document thread-safety of shared types
- **M-0634**: shinrai-orders: Add quickcheck/proptest where math-heavy
- **M-0635**: shinrai-orders: Review panic! sites; convert to Result

### polish:shinrai-paper

- **M-0636**: shinrai-paper: Add missing # Errors docs on pub fns
- **M-0637**: shinrai-paper: Add examples in crate-level docs
- **M-0638**: shinrai-paper: Review Clippy pedantic locally and fix safe lints
- **M-0639**: shinrai-paper: Increase unit test coverage for private helpers
- **M-0640**: shinrai-paper: Add Debug redaction for secret-bearing types
- **M-0641**: shinrai-paper: Stabilize public re-exports in lib.rs
- **M-0642**: shinrai-paper: Add integration test module outline
- **M-0643**: shinrai-paper: Document feature flags in Cargo.toml
- **M-0644**: shinrai-paper: Add criterion benches for hottest fn
- **M-0645**: shinrai-paper: Remove unused dependencies if any
- **M-0646**: shinrai-paper: Align error Display messages with HTTP codes
- **M-0647**: shinrai-paper: Add tracing events at module boundaries
- **M-0648**: shinrai-paper: Document thread-safety of shared types
- **M-0649**: shinrai-paper: Add quickcheck/proptest where math-heavy
- **M-0650**: shinrai-paper: Review panic! sites; convert to Result

### polish:shinrai-portfolio

- **M-0651**: shinrai-portfolio: Add missing # Errors docs on pub fns
- **M-0652**: shinrai-portfolio: Add examples in crate-level docs
- **M-0653**: shinrai-portfolio: Review Clippy pedantic locally and fix safe lints
- **M-0654**: shinrai-portfolio: Increase unit test coverage for private helpers
- **M-0655**: shinrai-portfolio: Add Debug redaction for secret-bearing types
- **M-0656**: shinrai-portfolio: Stabilize public re-exports in lib.rs
- **M-0657**: shinrai-portfolio: Add integration test module outline
- **M-0658**: shinrai-portfolio: Document feature flags in Cargo.toml
- **M-0659**: shinrai-portfolio: Add criterion benches for hottest fn
- **M-0660**: shinrai-portfolio: Remove unused dependencies if any
- **M-0661**: shinrai-portfolio: Align error Display messages with HTTP codes
- **M-0662**: shinrai-portfolio: Add tracing events at module boundaries
- **M-0663**: shinrai-portfolio: Document thread-safety of shared types
- **M-0664**: shinrai-portfolio: Add quickcheck/proptest where math-heavy
- **M-0665**: shinrai-portfolio: Review panic! sites; convert to Result

### polish:shinrai-risk

- **M-0666**: shinrai-risk: Add missing # Errors docs on pub fns
- **M-0667**: shinrai-risk: Add examples in crate-level docs
- **M-0668**: shinrai-risk: Review Clippy pedantic locally and fix safe lints
- **M-0669**: shinrai-risk: Increase unit test coverage for private helpers
- **M-0670**: shinrai-risk: Add Debug redaction for secret-bearing types
- **M-0671**: shinrai-risk: Stabilize public re-exports in lib.rs
- **M-0672**: shinrai-risk: Add integration test module outline
- **M-0673**: shinrai-risk: Document feature flags in Cargo.toml
- **M-0674**: shinrai-risk: Add criterion benches for hottest fn
- **M-0675**: shinrai-risk: Remove unused dependencies if any
- **M-0676**: shinrai-risk: Align error Display messages with HTTP codes
- **M-0677**: shinrai-risk: Add tracing events at module boundaries
- **M-0678**: shinrai-risk: Document thread-safety of shared types
- **M-0679**: shinrai-risk: Add quickcheck/proptest where math-heavy
- **M-0680**: shinrai-risk: Review panic! sites; convert to Result

### polish:shinrai-store

- **M-0681**: shinrai-store: Add missing # Errors docs on pub fns
- **M-0682**: shinrai-store: Add examples in crate-level docs
- **M-0683**: shinrai-store: Review Clippy pedantic locally and fix safe lints
- **M-0684**: shinrai-store: Increase unit test coverage for private helpers
- **M-0685**: shinrai-store: Add Debug redaction for secret-bearing types
- **M-0686**: shinrai-store: Stabilize public re-exports in lib.rs
- **M-0687**: shinrai-store: Add integration test module outline
- **M-0688**: shinrai-store: Document feature flags in Cargo.toml
- **M-0689**: shinrai-store: Add criterion benches for hottest fn
- **M-0690**: shinrai-store: Remove unused dependencies if any
- **M-0691**: shinrai-store: Align error Display messages with HTTP codes
- **M-0692**: shinrai-store: Add tracing events at module boundaries
- **M-0693**: shinrai-store: Document thread-safety of shared types
- **M-0694**: shinrai-store: Add quickcheck/proptest where math-heavy
- **M-0695**: shinrai-store: Review panic! sites; convert to Result

### polish:shinrai-telemetry

- **M-0696**: shinrai-telemetry: Add missing # Errors docs on pub fns
- **M-0697**: shinrai-telemetry: Add examples in crate-level docs
- **M-0698**: shinrai-telemetry: Review Clippy pedantic locally and fix safe lints
- **M-0699**: shinrai-telemetry: Increase unit test coverage for private helpers
- **M-0700**: shinrai-telemetry: Add Debug redaction for secret-bearing types
- **M-0701**: shinrai-telemetry: Stabilize public re-exports in lib.rs
- **M-0702**: shinrai-telemetry: Add integration test module outline
- **M-0703**: shinrai-telemetry: Document feature flags in Cargo.toml
- **M-0704**: shinrai-telemetry: Add criterion benches for hottest fn
- **M-0705**: shinrai-telemetry: Remove unused dependencies if any
- **M-0706**: shinrai-telemetry: Align error Display messages with HTTP codes
- **M-0707**: shinrai-telemetry: Add tracing events at module boundaries
- **M-0708**: shinrai-telemetry: Document thread-safety of shared types
- **M-0709**: shinrai-telemetry: Add quickcheck/proptest where math-heavy
- **M-0710**: shinrai-telemetry: Review panic! sites; convert to Result

### portfolio

- **M-0711**: Multi-currency equity / FX conversion
- **M-0712**: Short-position MTM / cost basis edges
- **M-0713**: MarkStore TTL / stale mark flags
- **M-0714**: Realized P&L day vs all-time in API
- **M-0715**: Divergence alarm book lots vs inventory
- **M-0716**: Streaming portfolio WebSocket
- **M-0717**: Benchmark snapshot with large history
- **M-0718**: Portfolio Greeks stub for options
- **M-0719**: Cash ladder (settled/unsettled)
- **M-0720**: Buying power displayed on portfolio
- **M-0721**: UI-ready condensed portfolio DTO
- **M-0722**: Mark source attribution (stored vs live)
- **M-0723**: Position lot-level tax lots stub
- **M-0724**: Nightly portfolio snapshot table
- **M-0725**: Property test: cash+positions reconcile ledger

### risk

- **M-0726**: ~~Persist kill switches / restrictions / overrides~~ **done** (`risk_state`; overrides via approvals hydrate)
- **M-0727**: ~~Fail closed when collar_bps>0 and ref_price None~~ **done**
- **M-0728**: Per-symbol and per-venue limits
- **M-0729**: Gross and net notional portfolio limits
- **M-0730**: Order-frequency / rate-of-trading rule
- **M-0731**: Single-name concentration limits
- **M-0732**: Market-hours calendars (holidays/early close)
- **M-0733**: Day P&L reset persistence across restarts
- **M-0734**: ~~Ops audit trail for every risk mutation~~ **done** (`risk_ops_audit` + `GET /v1/ops/risk-audit`)
- **M-0735**: Reject negative/zero patched limits in ops HTTP
- **M-0736**: Short locate / borrow check stub
- **M-0737**: Asset-class exposure multi-currency edge cases
- **M-0738**: Proptest all RiskRejectReason codes
- **M-0739**: Bad SHINRAI_RISK_MARKET_SESSION fails startup
- **M-0740**: Buying-power includes unsettled cash flag
- **M-0741**: Options buying power model stub
- **M-0742**: Fat-finger max order qty rule
- **M-0743**: Price band absolute ticks rule
- **M-0744**: Restricted list hot-reload
- **M-0745**: Risk reason → metrics label map
- **M-0746**: Per-account risk limit overlays
- **M-0747**: Global kill clears via dual control
- **M-0748**: Risk engine snapshot for hydrate
- **M-0749**: Integration test: day-loss trip mid-session
- **M-0750**: Document risk precedence order

### store

- **M-0751**: CHECK constraints for status enums
- **M-0752**: Index strategy review under load
- **M-0753**: Soft-delete / archive orders
- **M-0754**: Inbox retention / purge job
- **M-0755**: Outbox payload size limits
- **M-0756**: Transactional integrity mid-failure tests
- **M-0757**: Pool metrics / timeouts
- **M-0758**: Read replicas / statement timeouts
- **M-0759**: Never leak SQL to clients
- **M-0760**: Hash-chain backfill verification tool
- **M-0761**: positions.reserved_lots SQL consistency checks
- **M-0762**: Drop-copy uniqueness constraints
- **M-0763**: Approvals dual-control DB invariants
- **M-0764**: Expand postgres.rs coverage
- **M-0765**: Vacuum/bloat guidance in deployment.md
- **M-0766**: Migration down scripts policy
- **M-0767**: sqlx offline data checked in
- **M-0768**: Partition audit_records by time
- **M-0769**: Store encryption-at-rest guidance
- **M-0770**: Backup/restore integration test

### telemetry

- **M-0771**: Prometheus/OTLP metrics pipeline
- **M-0772**: Resource attrs: version + environment
- **M-0773**: Disable ANSI when not a TTY
- **M-0774**: Sampling for high-cardinality spans
- **M-0775**: Exporter buffer-full health
- **M-0776**: Document second-init empty guard
- **M-0777**: Correlate logs↔traces via trace_id
- **M-0778**: Runtime Tokio metrics export
- **M-0779**: Panic hook with tracing
- **M-0780**: Build info span attributes

### testing

- **M-0781**: Proptest risk collar at tick boundaries
- **M-0782**: Replace hydrate coverage in dual_write
- **M-0783**: Expand FIX venue gateway tests
- **M-0784**: Licensed venue timeout tests at gateway
- **M-0785**: Funds step-up threshold tests
- **M-0786**: Ops allowlist spoofing tests
- **M-0787**: Retry-After rate limit header tests
- **M-0788**: Audit export HMAC tests
- **M-0789**: Recon broker fill mismatch fixtures
- **M-0790**: Windows script path docs
- **M-0791**: Load test harness (k6/vegeta)
- **M-0792**: Mutation testing on OMS transitions
- **M-0793**: OpenAPI contract tests
- **M-0794**: Ignored-live secrets matrix in testing.md
- **M-0795**: Coverage report in CI
- **M-0796**: Flake quarantine workflow
- **M-0797**: Benchmark gate on PR for hot paths
- **M-0798**: UI screenshot regression
- **M-0799**: Fuzz OG JSON order body
- **M-0800**: Chaos mesh / toxiproxy recipe

