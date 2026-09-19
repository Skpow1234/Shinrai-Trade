//! Axum router, shared state, and env configuration.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use shinrai_exchange_simulator::FaultConfig;
use shinrai_execution::{LicensedSandboxConfig, SandboxConfig};
use shinrai_instruments::{phase1_master, InstrumentMaster};
use shinrai_ledger::AccountId;
use shinrai_md_fanout::{FanoutError, SubjectId, TokenAuth, TokenTtl};
use shinrai_money::{Currency, Money};
use shinrai_orders::Order;
use shinrai_paper::{PaperEngine, VenueKind};
use shinrai_portfolio::MarkStore;
use shinrai_risk::{RiskEngine, RiskLimits};
use shinrai_store::StorePool;
use tower_http::trace::TraceLayer;

/// Coarse gateway counters for local ops (not billing-grade).
#[derive(Debug, Default)]
#[allow(clippy::struct_field_names)]
pub struct GatewayMetrics {
    orders_submitted: AtomicU64,
    orders_accepted: AtomicU64,
    orders_risk_rejected: AtomicU64,
    orders_canceled: AtomicU64,
    dual_write_failures: AtomicU64,
    persist_degraded: AtomicU64,
    risk_reject_codes: Mutex<HashMap<String, u64>>,
}

impl GatewayMetrics {
    /// Records an order submit attempt.
    pub fn record_submit(&self) {
        self.orders_submitted.fetch_add(1, Ordering::Relaxed);
    }

    /// Records an accepted order (created or duplicate return).
    pub fn record_accepted(&self) {
        self.orders_accepted.fetch_add(1, Ordering::Relaxed);
    }

    /// Records a pre-trade risk rejection.
    pub fn record_risk_rejected(&self) {
        self.orders_risk_rejected.fetch_add(1, Ordering::Relaxed);
    }

    /// Records a risk rejection with a stable reason code.
    pub fn record_risk_rejected_code(&self, code: &str) {
        self.record_risk_rejected();
        let mut map = self
            .risk_reject_codes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *map.entry(code.to_owned()).or_insert(0) += 1;
    }

    /// Records a cancel that reached the OMS path.
    pub fn record_canceled(&self) {
        self.orders_canceled.fetch_add(1, Ordering::Relaxed);
    }

    /// Records a required persist failure (store is authoritative when configured).
    pub fn record_dual_write_failure(&self) {
        self.dual_write_failures.fetch_add(1, Ordering::Relaxed);
        self.persist_degraded.store(1, Ordering::Release);
    }

    /// Whether the gateway has entered persist-degraded mode (kill switch engaged).
    #[must_use]
    pub fn is_persist_degraded(&self) -> bool {
        self.persist_degraded.load(Ordering::Acquire) != 0
    }

    /// JSON snapshot of atomic counters (no OMS lock).
    #[must_use]
    pub fn snapshot(&self) -> Value {
        let risk_by_code = self
            .risk_reject_codes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        json!({
            "orders_submitted": self.orders_submitted.load(Ordering::Relaxed),
            "orders_accepted": self.orders_accepted.load(Ordering::Relaxed),
            "orders_risk_rejected": self.orders_risk_rejected.load(Ordering::Relaxed),
            "orders_canceled": self.orders_canceled.load(Ordering::Relaxed),
            "dual_write_failures": self.dual_write_failures.load(Ordering::Relaxed),
            "persist_degraded": self.is_persist_degraded(),
            "risk_rejects_by_code": risk_by_code,
        })
    }
}

/// Shared gateway state (mutex only at this I/O edge).
#[derive(Clone)]
pub struct AppState {
    pub(crate) engine: Arc<Mutex<PaperEngine>>,
    pub(crate) auth: TokenAuth,
    pub(crate) accounts: Arc<HashMap<String, AccountId>>,
    pub(crate) master: InstrumentMaster,
    pub(crate) metrics: Arc<GatewayMetrics>,
    pub(crate) marks: Arc<Mutex<MarkStore>>,
    pub(crate) md_base_url: Option<String>,
    pub(crate) md_token: Option<String>,
    /// Optional Postgres write-through (set when `SHINRAI_DATABASE_URL` is configured).
    pub(crate) store: Option<StorePool>,
    /// Highest audit `seq` successfully persisted (watermark).
    pub(crate) audit_persisted_seq: Arc<AtomicU64>,
    /// Stuck pending age threshold (logical seconds).
    pub(crate) stuck_age_secs: u64,
    /// Outbox publisher counters (shared with background task).
    pub(crate) outbox_metrics: Arc<crate::outbox_publisher::OutboxMetrics>,
    /// Per-subject submit rate limiter.
    pub(crate) rate_limiter: Arc<crate::rate_limit::RateLimiter>,
    /// Optional ops bearer token (when set, `/v1/ops*` and `/v1/metrics` require it).
    pub(crate) ops_token: Option<String>,
    /// When non-empty, ops/metrics only from these client IPs / CIDRs.
    pub(crate) ops_allowlist: Vec<crate::ops_allowlist::AllowEntry>,
    /// Shared MD HTTP client (optional mTLS).
    pub(crate) md_client: Arc<crate::md_client::MdHttpClient>,
    /// KYC gate (env map and/or HTTP vendor).
    pub(crate) kyc: crate::compliance::KycGate,
    /// Dual-control restricted-instrument approvals.
    pub(crate) approvals: crate::compliance::SharedApprovals,
    /// Optional admin override bearer for restricted instruments.
    pub(crate) admin_override_token: Option<String>,
    /// When false with a store attached, persist errors are logged but not returned.
    pub(crate) store_fail_hard: bool,
    /// Paper fund movement APIs enabled.
    pub(crate) funds_enabled: bool,
    /// Paper withdrawals enabled (deposits may still be on).
    pub(crate) withdrawals_enabled: bool,
    /// When set, paper withdrawals ≥ this minor amount need `X-Admin-Override`.
    pub(crate) withdraw_approval_threshold_minor: Option<i128>,
    /// Last injected broker EOD snapshot (ops).
    pub(crate) eod_snapshot: Arc<Mutex<Option<shinrai_paper::BrokerEodSnapshot>>>,
}

/// Process configuration (tokens / secrets are not displayed).
#[derive(Clone)]
pub struct GatewayConfig {
    static_tokens: Vec<(String, String)>,
    clients: Vec<(String, String, String)>,
    accounts: Vec<(String, u64)>,
    deposits: Vec<(u64, i64)>,
    ttl: TokenTtl,
    bootstrap_marks: Vec<(String, i64)>,
    md_base_url: Option<String>,
    md_token: Option<String>,
    stuck_age_secs: u64,
    venue_kind: VenueKind,
    ops_token: Option<String>,
    risk_limits: RiskLimits,
    /// Remote REST venue URL when `venue_kind == Rest` (local when unset).
    rest_url: Option<String>,
    rest_bearer: Option<String>,
    ops_allowlist_raw: Option<String>,
    /// Comma-separated restricted symbols at bootstrap.
    restricted_symbols: Vec<String>,
    kyc_raw: Option<String>,
    admin_override_token: Option<String>,
    /// Fail-hard on store write (default true).
    store_fail_hard: bool,
    /// Force local Alpaca mock even if env credentials exist (tests).
    alpaca_force_local: bool,
    /// Enable paper deposit/withdraw HTTP APIs (default true).
    funds_enabled: bool,
    /// Enable paper withdrawals (default true).
    withdrawals_enabled: bool,
    /// Optional large-withdraw step-up threshold (minor units).
    withdraw_approval_threshold_minor: Option<i128>,
}

impl GatewayConfig {
    /// Builds a config. Empty clients and static tokens is fail-closed for auth.
    #[must_use]
    pub fn new(
        static_tokens: Vec<(String, String)>,
        clients: Vec<(String, String, String)>,
        accounts: Vec<(String, u64)>,
        deposits: Vec<(u64, i64)>,
        ttl: TokenTtl,
    ) -> Self {
        Self {
            static_tokens,
            clients,
            accounts,
            deposits,
            ttl,
            bootstrap_marks: Vec::new(),
            md_base_url: None,
            md_token: None,
            stuck_age_secs: crate::ops::DEFAULT_STUCK_AGE_SECS,
            venue_kind: VenueKind::Sim,
            ops_token: None,
            risk_limits: RiskLimits::demo(),
            rest_url: None,
            rest_bearer: None,
            ops_allowlist_raw: None,
            restricted_symbols: Vec::new(),
            kyc_raw: None,
            admin_override_token: None,
            store_fail_hard: true,
            alpaca_force_local: false,
            funds_enabled: true,
            withdrawals_enabled: true,
            withdraw_approval_threshold_minor: None,
        }
    }

    /// Reads env: `SHINRAI_OG_*` variables.
    #[must_use]
    pub fn from_env() -> Self {
        let access = env_u64("SHINRAI_OG_ACCESS_TTL").unwrap_or(60);
        let refresh = env_u64("SHINRAI_OG_REFRESH_TTL").unwrap_or(3_600);
        let mut cfg = Self::new(
            parse_tokens(std::env::var("SHINRAI_OG_TOKENS").ok().as_deref()),
            parse_clients(std::env::var("SHINRAI_OG_CLIENTS").ok().as_deref()),
            parse_accounts(std::env::var("SHINRAI_OG_ACCOUNTS").ok().as_deref()),
            parse_deposits(std::env::var("SHINRAI_OG_DEPOSITS").ok().as_deref()),
            TokenTtl::new(access, refresh),
        );
        cfg.bootstrap_marks = parse_symbol_marks(std::env::var("SHINRAI_OG_MARKS").ok().as_deref());
        cfg.md_base_url = std::env::var("SHINRAI_OG_MD_URL")
            .ok()
            .filter(|s| !s.is_empty());
        cfg.md_token = std::env::var("SHINRAI_OG_MD_TOKEN")
            .ok()
            .filter(|s| !s.is_empty());
        if let Some(age) = env_u64("SHINRAI_OG_STUCK_AGE_SECS") {
            cfg.stuck_age_secs = age;
        }
        cfg.venue_kind = parse_venue_kind(std::env::var("SHINRAI_OG_VENUE").ok().as_deref());
        cfg.ops_token = std::env::var("SHINRAI_OG_OPS_TOKEN")
            .ok()
            .filter(|s| !s.is_empty());
        cfg.risk_limits = RiskLimits::demo_from_env();
        cfg.rest_url = std::env::var("SHINRAI_OG_REST_URL")
            .ok()
            .filter(|s| !s.is_empty());
        cfg.rest_bearer = std::env::var("SHINRAI_OG_REST_TOKEN")
            .ok()
            .filter(|s| !s.is_empty());
        cfg.ops_allowlist_raw = std::env::var("SHINRAI_OG_OPS_ALLOWLIST")
            .ok()
            .filter(|s| !s.is_empty());
        cfg.restricted_symbols = parse_csv_list(
            std::env::var("SHINRAI_OG_RESTRICTED_SYMBOLS")
                .ok()
                .as_deref(),
        );
        cfg.kyc_raw = std::env::var("SHINRAI_OG_KYC_STATUS")
            .ok()
            .filter(|s| !s.is_empty());
        cfg.admin_override_token = std::env::var("SHINRAI_OG_ADMIN_OVERRIDE_TOKEN")
            .ok()
            .filter(|s| !s.is_empty());
        if let Ok(v) = std::env::var("SHINRAI_OG_STORE_FAIL_HARD") {
            cfg.store_fail_hard = !matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "0" | "false" | "no" | "off"
            );
        }
        if let Ok(v) = std::env::var("SHINRAI_OG_FUNDS_ENABLED") {
            cfg.funds_enabled = !matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "0" | "false" | "no" | "off"
            );
        }
        if let Ok(v) = std::env::var("SHINRAI_OG_WITHDRAWALS_ENABLED") {
            cfg.withdrawals_enabled = !matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "0" | "false" | "no" | "off"
            );
        }
        cfg.withdraw_approval_threshold_minor =
            std::env::var("SHINRAI_OG_WITHDRAW_APPROVAL_THRESHOLD_MINOR")
                .ok()
                .and_then(|s| s.parse().ok())
                .filter(|n: &i128| *n > 0);
        cfg
    }
}

impl core::fmt::Debug for GatewayConfig {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GatewayConfig")
            .field("static_token_entries", &self.static_tokens.len())
            .field("client_entries", &self.clients.len())
            .field("account_entries", &self.accounts.len())
            .field("deposit_entries", &self.deposits.len())
            .field("ttl", &self.ttl)
            .field("bootstrap_mark_entries", &self.bootstrap_marks.len())
            .field("md_base_url_configured", &self.md_base_url.is_some())
            .field("md_token_configured", &self.md_token.is_some())
            .field("stuck_age_secs", &self.stuck_age_secs)
            .field("venue_kind", &self.venue_kind)
            .field("ops_token_configured", &self.ops_token.is_some())
            .field("risk_collar_bps", &self.risk_limits.collar_bps)
            .field(
                "risk_max_daily_loss_minor",
                &self.risk_limits.max_daily_loss_minor,
            )
            .field("rest_url_configured", &self.rest_url.is_some())
            .field("rest_bearer_configured", &self.rest_bearer.is_some())
            .field(
                "ops_allowlist_configured",
                &self.ops_allowlist_raw.is_some(),
            )
            .field("restricted_symbol_entries", &self.restricted_symbols.len())
            .field("kyc_configured", &self.kyc_raw.is_some())
            .field(
                "admin_override_configured",
                &self.admin_override_token.is_some(),
            )
            .field("store_fail_hard", &self.store_fail_hard)
            .field("alpaca_force_local", &self.alpaca_force_local)
            .field("funds_enabled", &self.funds_enabled)
            .field("withdrawals_enabled", &self.withdrawals_enabled)
            .field(
                "withdraw_approval_threshold_minor",
                &self.withdraw_approval_threshold_minor,
            )
            .finish_non_exhaustive()
    }
}

impl AppState {
    /// Builds state from config.
    #[must_use]
    #[allow(clippy::too_many_lines)]
    pub fn from_config(config: &GatewayConfig) -> Self {
        let auth = TokenAuth::new(config.ttl);
        for (token, subject) in &config.static_tokens {
            auth.grant_static_access(token, SubjectId::new(subject.clone()));
        }
        for (id, secret, subject) in &config.clients {
            auth.register_client(id, secret, SubjectId::new(subject.clone()));
        }

        let accounts: HashMap<String, AccountId> = config
            .accounts
            .iter()
            .map(|(subject, raw)| (subject.clone(), AccountId::from_u64(*raw)))
            .collect();

        let master = phase1_master();
        let mut risk = RiskEngine::new(config.risk_limits);
        for sym in &config.restricted_symbols {
            if let Ok(alias) = shinrai_instruments::ExternalId::ticker(sym) {
                if let Ok(id) = master.resolve_alias(&alias) {
                    risk.restrict_instrument(id);
                }
            }
        }
        let mut engine = match config.venue_kind {
            VenueKind::Sim => {
                PaperEngine::with_risk(master.clone(), FaultConfig::happy_path(), risk.clone())
            }
            VenueKind::Sandbox => {
                PaperEngine::with_sandbox(master.clone(), SandboxConfig::happy_path(), risk.clone())
            }
            VenueKind::Rest => {
                if let Some(url) = config.rest_url.as_deref() {
                    match PaperEngine::with_rest_remote(
                        master.clone(),
                        risk.clone(),
                        url,
                        config.rest_bearer.clone(),
                    ) {
                        Ok(e) => e,
                        Err(err) => {
                            eprintln!(
                                "shinrai-order-gateway: remote REST venue failed ({err}); using local paper REST"
                            );
                            PaperEngine::with_rest(master.clone(), risk.clone())
                        }
                    }
                } else {
                    PaperEngine::with_rest(master.clone(), risk.clone())
                }
            }
            VenueKind::Licensed => PaperEngine::with_licensed(
                master.clone(),
                LicensedSandboxConfig::happy_path(),
                risk.clone(),
            ),
            VenueKind::Alpaca => {
                let symbols: HashMap<_, _> = master
                    .iter()
                    .map(|i| (i.id(), i.symbol_display().to_owned()))
                    .collect();
                if config.alpaca_force_local {
                    PaperEngine::with_alpaca(master.clone(), risk.clone(), symbols)
                } else if let Some(cfg) = shinrai_execution::AlpacaConfig::from_env() {
                    match PaperEngine::with_alpaca_remote(
                        master.clone(),
                        risk.clone(),
                        cfg,
                        symbols,
                    ) {
                        Ok(e) => e,
                        Err(err) => {
                            panic!(
                                "shinrai-order-gateway: Alpaca remote required but failed: {err}"
                            );
                        }
                    }
                } else {
                    PaperEngine::with_alpaca(master.clone(), risk.clone(), symbols)
                }
            }
        };

        for (account_raw, major) in &config.deposits {
            let account = AccountId::from_u64(*account_raw);
            if let Ok(amount) = Money::from_major(i128::from(*major), Currency::usd()) {
                let _ = engine.deposit(account, amount, format!("bootstrap:{account_raw}"));
            }
        }

        let mut marks = MarkStore::new();
        for (symbol, scaled) in &config.bootstrap_marks {
            if let Ok(alias) = shinrai_instruments::ExternalId::ticker(symbol) {
                if let Ok(id) = master.resolve_alias(&alias) {
                    let px = shinrai_instruments::PriceTicks::from_scaled(*scaled);
                    marks.set(id, px);
                    engine.set_mark(id, px);
                }
            }
        }

        Self {
            engine: Arc::new(Mutex::new(engine)),
            auth,
            accounts: Arc::new(accounts),
            master,
            metrics: Arc::new(GatewayMetrics::default()),
            marks: Arc::new(Mutex::new(marks)),
            md_base_url: config.md_base_url.clone(),
            md_token: config.md_token.clone(),
            store: None,
            audit_persisted_seq: Arc::new(AtomicU64::new(0)),
            stuck_age_secs: config.stuck_age_secs,
            outbox_metrics: Arc::new(crate::outbox_publisher::OutboxMetrics::default()),
            rate_limiter: Arc::new(crate::rate_limit::RateLimiter::demo()),
            ops_token: config.ops_token.clone(),
            ops_allowlist: crate::ops_allowlist::parse_allowlist(
                config.ops_allowlist_raw.as_deref(),
            ),
            md_client: Arc::new(crate::md_client::MdHttpClient::from_env()),
            kyc: crate::compliance::KycGate::from_env(config.kyc_raw.as_deref()),
            approvals: Arc::new(Mutex::new(crate::compliance::ApprovalStore::new())),
            admin_override_token: config.admin_override_token.clone(),
            store_fail_hard: config.store_fail_hard,
            funds_enabled: config.funds_enabled,
            withdrawals_enabled: config.withdrawals_enabled,
            withdraw_approval_threshold_minor: config.withdraw_approval_threshold_minor,
            eod_snapshot: Arc::new(Mutex::new(None)),
        }
    }

    /// Attaches a Postgres pool for write-through after trading mutations.
    pub async fn attach_store(&mut self, pool: StorePool) {
        if let Ok(orders) = shinrai_store::list_orders(&pool).await {
            let max_id = orders.iter().map(|o| o.id.get()).max().unwrap_or(0);
            if max_id > 0 {
                let mut engine = lock_engine(self);
                engine.bump_order_ids_past(max_id);
            }
        }
        self.store = Some(pool);
    }

    /// Hydrates the in-memory engine from Postgres (startup replay).
    ///
    /// # Errors
    ///
    /// Returns store / decode errors.
    pub async fn hydrate_from_store(
        &mut self,
        pool: StorePool,
    ) -> Result<(), shinrai_store::StoreError> {
        let payload = crate::hydrate::load_hydrate_payload(&pool).await?;
        let max_seq = {
            let mut engine = lock_engine(self);
            crate::hydrate::apply_hydrate(&mut engine, payload)?
        };
        self.store = Some(pool);
        self.audit_persisted_seq.store(max_seq, Ordering::Release);
        Ok(())
    }

    /// Persists bootstrap ledger/audit (deposits) after `attach_store`.
    ///
    /// # Errors
    ///
    /// Returns store errors when write-through fails.
    pub async fn persist_bootstrap(&self) -> Result<(), shinrai_store::StoreError> {
        self.must_persist(None).await
    }

    /// Required write-through of order (if any), full ledger journal, and new audit rows.
    ///
    /// When no store is configured, this is a no-op. On failure: engages the global
    /// kill switch, marks persist degraded, and returns the store error so HTTP
    /// handlers can refuse to ack the client.
    ///
    /// # Errors
    ///
    /// Returns [`shinrai_store::StoreError`] when Postgres write fails.
    pub async fn must_persist(
        &self,
        order: Option<&Order>,
    ) -> Result<(), shinrai_store::StoreError> {
        let Some(pool) = &self.store else {
            return Ok(());
        };
        let after = self.audit_persisted_seq.load(Ordering::Acquire);
        let batch = {
            let engine = lock_engine(self);
            crate::persist::collect_batch(&engine, order, after)
        };
        let max_seq = batch
            .audit
            .iter()
            .map(shinrai_audit::AuditRecord::seq)
            .max();
        match crate::persist::write_batch(pool, &batch).await {
            Ok(()) => {
                if let Some(seq) = max_seq {
                    self.audit_persisted_seq.fetch_max(seq, Ordering::Release);
                }
                Ok(())
            }
            Err(err) => {
                self.metrics.record_dual_write_failure();
                {
                    let mut engine = lock_engine(self);
                    engine.risk_mut().set_global_kill(true);
                }
                eprintln!("shinrai-order-gateway: persist failed (kill switch engaged): {err}");
                if self.store_fail_hard {
                    Err(err)
                } else {
                    Ok(())
                }
            }
        }
    }

    /// Restores engine memory from a pre-mutate snapshot, then engages kill.
    pub fn rollback_engine(&self, snapshot: PaperEngine) {
        let mut engine = lock_engine(self);
        engine.restore_from(snapshot);
        engine.risk_mut().set_global_kill(true);
    }

    /// Clones the current engine (for persist-failure rollback).
    #[must_use]
    pub fn snapshot_engine(&self) -> PaperEngine {
        lock_engine(self).clone()
    }

    /// Shared outbox metrics handle (for background publisher).
    #[must_use]
    pub fn outbox_metrics(&self) -> Arc<crate::outbox_publisher::OutboxMetrics> {
        Arc::clone(&self.outbox_metrics)
    }

    /// Optional store pool clone for background tasks.
    #[must_use]
    pub fn store_pool(&self) -> Option<StorePool> {
        self.store.clone()
    }

    /// Closes the Postgres pool so the next `must_persist` fails (chaos tests).
    pub async fn close_store_for_test(&mut self) {
        if let Some(pool) = self.store.take() {
            pool.close().await;
            // Keep a closed pool so write-through still attempts and fails.
            self.store = Some(pool);
        }
    }

    /// Sets an ops bearer for `/v1/ops*` and `/v1/metrics` (tests).
    #[must_use]
    pub fn with_ops_token(mut self, token: &str) -> Self {
        self.ops_token = Some(token.to_owned());
        self
    }

    /// Replaces the submit rate limiter (tests).
    #[must_use]
    pub fn with_rate_limiter(mut self, limiter: crate::rate_limit::RateLimiter) -> Self {
        self.rate_limiter = Arc::new(limiter);
        self
    }

    /// Disconnects the paper venue (ambiguous mid-flight / stuck `PendingNew` tests).
    pub fn disconnect_venue_for_test(&self) {
        let mut engine = lock_engine(self);
        engine.disconnect_venue();
    }

    /// Test helper with a single static access token and one mapped account.
    #[must_use]
    pub fn for_test(token: &str, subject: &str, account: u64, deposit_major: i64) -> Self {
        Self::from_config(&GatewayConfig::new(
            vec![(token.to_owned(), subject.to_owned())],
            Vec::new(),
            vec![(subject.to_owned(), account)],
            vec![(account, deposit_major)],
            TokenTtl::default(),
        ))
    }

    /// Test helper with sandbox venue (ack + auto-fill).
    #[must_use]
    pub fn for_test_sandbox(token: &str, subject: &str, account: u64, deposit_major: i64) -> Self {
        let mut cfg = GatewayConfig::new(
            vec![(token.to_owned(), subject.to_owned())],
            Vec::new(),
            vec![(subject.to_owned(), account)],
            vec![(account, deposit_major)],
            TokenTtl::default(),
        );
        cfg.venue_kind = VenueKind::Sandbox;
        Self::from_config(&cfg)
    }

    /// Test helper with local Alpaca paper mock (no network).
    #[must_use]
    pub fn for_test_alpaca(token: &str, subject: &str, account: u64, deposit_major: i64) -> Self {
        let mut cfg = GatewayConfig::new(
            vec![(token.to_owned(), subject.to_owned())],
            Vec::new(),
            vec![(subject.to_owned(), account)],
            vec![(account, deposit_major)],
            TokenTtl::default(),
        );
        cfg.venue_kind = VenueKind::Alpaca;
        cfg.alpaca_force_local = true;
        Self::from_config(&cfg)
    }

    /// Test helper with REST paper venue (JSON local broker + auto-fill).
    #[must_use]
    pub fn for_test_rest(token: &str, subject: &str, account: u64, deposit_major: i64) -> Self {
        let mut cfg = GatewayConfig::new(
            vec![(token.to_owned(), subject.to_owned())],
            Vec::new(),
            vec![(subject.to_owned(), account)],
            vec![(account, deposit_major)],
            TokenTtl::default(),
        );
        cfg.venue_kind = VenueKind::Rest;
        Self::from_config(&cfg)
    }

    /// Test helper with sim venue resting fills (no auto-fill).
    #[must_use]
    pub fn for_test_resting(token: &str, subject: &str, account: u64, deposit_major: i64) -> Self {
        use shinrai_exchange_simulator::{FaultConfig, FillPolicy};
        let state = Self::for_test(token, subject, account, deposit_major);
        {
            let mut engine = lock_engine(&state);
            engine.set_sim_faults(FaultConfig {
                fill_policy: FillPolicy::Rest,
                ..FaultConfig::happy_path()
            });
        }
        state
    }

    /// Test helper with write-through to Postgres (caller migrates the pool).
    pub async fn for_test_with_store(
        token: &str,
        subject: &str,
        account: u64,
        deposit_major: i64,
        pool: StorePool,
    ) -> Self {
        let mut state = Self::for_test(token, subject, account, deposit_major);
        state.attach_store(pool).await;
        state.persist_bootstrap().await.expect("persist bootstrap");
        state
    }

    /// Test helper: sim venue with [`FillPolicy::Rest`] + Postgres write-through.
    pub async fn for_test_with_store_resting(
        token: &str,
        subject: &str,
        account: u64,
        deposit_major: i64,
        pool: StorePool,
    ) -> Self {
        use shinrai_exchange_simulator::{FaultConfig, FillPolicy};
        let mut state = Self::for_test(token, subject, account, deposit_major);
        {
            let mut engine = lock_engine(&state);
            engine.set_sim_faults(FaultConfig {
                fill_policy: FillPolicy::Rest,
                ..FaultConfig::happy_path()
            });
        }
        state.attach_store(pool).await;
        state.persist_bootstrap().await.expect("persist bootstrap");
        state
    }

    /// Test helper: rebuild state from Postgres only (no env deposits).
    pub async fn for_test_hydrate(
        token: &str,
        subject: &str,
        account: u64,
        pool: StorePool,
    ) -> Self {
        let mut state = Self::from_config(&GatewayConfig::new(
            vec![(token.to_owned(), subject.to_owned())],
            Vec::new(),
            vec![(subject.to_owned(), account)],
            Vec::new(),
            TokenTtl::default(),
        ));
        // Resting venue so hydrate-restored working orders are not auto-filled.
        {
            use shinrai_exchange_simulator::{FaultConfig, FillPolicy};
            let mut engine = lock_engine(&state);
            engine.set_sim_faults(FaultConfig {
                fill_policy: FillPolicy::Rest,
                ..FaultConfig::happy_path()
            });
        }
        state
            .hydrate_from_store(pool)
            .await
            .expect("hydrate from store");
        state
    }

    /// Test helper with a single stuck `PendingNew` order (no venue progress).
    #[must_use]
    pub fn for_test_with_stuck_pending(
        token: &str,
        subject: &str,
        account: u64,
        order_id: u64,
    ) -> Self {
        let state = Self::from_config(&GatewayConfig::new(
            vec![(token.to_owned(), subject.to_owned())],
            Vec::new(),
            vec![(subject.to_owned(), account)],
            Vec::new(),
            TokenTtl::default(),
        ));
        let order = shinrai_orders::Order::restore(
            shinrai_orders::OrderId::from_u64(order_id),
            AccountId::from_u64(account),
            shinrai_orders::ClientOrderId::new(format!("stuck-{order_id}")).expect("clid"),
            shinrai_instruments::InstrumentId::from_u64(1),
            shinrai_orders::Side::Buy,
            shinrai_orders::OrderType::Limit,
            shinrai_orders::TimeInForce::Gtc,
            shinrai_orders::OrderStatus::PendingNew,
            shinrai_instruments::QuantityLots::from_lots(1),
            shinrai_instruments::PriceTicks::from_scaled(100),
            shinrai_instruments::QuantityLots::from_lots(0),
            shinrai_instruments::QuantityLots::from_lots(1),
            None,
            None,
            None,
            Vec::new(),
        )
        .expect("order");
        {
            let mut engine = lock_engine(&state);
            engine
                .hydrate(Vec::new(), vec![order], Vec::new(), Vec::new())
                .expect("hydrate stuck");
        }
        state
    }

    /// Test helper with MD gateway URL for live portfolio marks.
    #[must_use]
    pub fn for_test_with_md(
        token: &str,
        subject: &str,
        account: u64,
        deposit_major: i64,
        md_base_url: &str,
        md_token: &str,
    ) -> Self {
        let mut cfg = GatewayConfig::new(
            vec![(token.to_owned(), subject.to_owned())],
            Vec::new(),
            vec![(subject.to_owned(), account)],
            vec![(account, deposit_major)],
            TokenTtl::default(),
        );
        cfg.md_base_url = Some(md_base_url.to_owned());
        cfg.md_token = Some(md_token.to_owned());
        Self::from_config(&cfg)
    }

    /// Test helper with client credentials.
    #[must_use]
    pub fn for_test_client(
        client_id: &str,
        secret: &str,
        subject: &str,
        account: u64,
        deposit_major: i64,
    ) -> Self {
        Self::from_config(&GatewayConfig::new(
            Vec::new(),
            vec![(client_id.to_owned(), secret.to_owned(), subject.to_owned())],
            vec![(subject.to_owned(), account)],
            vec![(account, deposit_major)],
            TokenTtl::new(60, 3_600),
        ))
    }
}

/// Logical clock (unix seconds).
#[must_use]
pub fn unix_logical_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// HTTP router.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v1/auth/token", post(crate::auth_http::post_token))
        .route("/v1/auth/revoke", post(crate::auth_http::post_revoke))
        .route(
            "/v1/orders",
            get(crate::orders_http::list_orders).post(crate::orders_http::post_order),
        )
        .route("/v1/orders/{id}", get(crate::orders_http::get_order))
        .route(
            "/v1/orders/{id}/cancel",
            post(crate::orders_http::post_cancel),
        )
        .route(
            "/v1/orders/{id}/replace",
            post(crate::orders_http::post_replace),
        )
        .route("/v1/portfolio", get(crate::portfolio_http::get_portfolio))
        .route("/v1/audit", get(crate::portfolio_http::get_audit))
        .route(
            "/v1/accounts/balances",
            get(crate::funds_http::get_balances),
        )
        .route(
            "/v1/accounts/deposit",
            post(crate::funds_http::post_deposit),
        )
        .route(
            "/v1/accounts/withdraw",
            post(crate::funds_http::post_withdraw),
        )
        .route(
            "/v1/reconciliation",
            get(crate::portfolio_http::get_reconciliation),
        )
        .route("/v1/metrics", get(crate::portfolio_http::get_metrics))
        .route("/v1/ops", get(crate::ops_http::get_ops_dashboard))
        .route(
            "/v1/ops/stuck-orders",
            get(crate::ops_http::get_stuck_orders),
        )
        .route(
            "/v1/ops/risk",
            get(crate::ops_http::get_risk).post(crate::ops_http::post_risk),
        )
        .route(
            "/v1/ops/reconciliation/eod",
            post(crate::ops_http::post_eod_reconciliation),
        )
        .route(
            "/v1/ops/approvals",
            get(crate::ops_http::get_approvals).post(crate::ops_http::post_approvals),
        )
        .route(
            "/v1/ops/audit/export",
            get(crate::ops_http::get_audit_export),
        )
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn health() -> Json<Value> {
    Json(json!({ "status": "ok", "service": "order-gateway" }))
}

/// Token query param for REST auth.
#[derive(Debug, Deserialize)]
pub struct AuthQuery {
    /// Bearer access token (query fallback).
    pub token: Option<String>,
}

pub(crate) fn unauthorized(err: FanoutError) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({ "type": "error", "code": err.code() })),
    )
        .into_response()
}

pub(crate) fn extract_bearer(headers: &HeaderMap, query: &AuthQuery) -> Option<String> {
    if let Some(value) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    {
        if let Some((scheme, rest)) = value.split_once(' ') {
            if scheme.eq_ignore_ascii_case("bearer") {
                let rest = rest.trim();
                if !rest.is_empty() {
                    return Some(rest.to_owned());
                }
            }
        }
    }
    query
        .token
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// When `SHINRAI_OG_OPS_TOKEN` / allowlist is set, ops/metrics require auth.
#[allow(clippy::result_large_err)]
pub(crate) fn require_ops_auth(
    state: &AppState,
    headers: &HeaderMap,
    ops_token_query: Option<&str>,
    peer_ip: Option<std::net::IpAddr>,
) -> Result<(), Response> {
    if !state.ops_allowlist.is_empty() {
        let Some(client) = crate::ops_allowlist::client_ip_from_headers(headers).or(peer_ip) else {
            return Err((
                StatusCode::FORBIDDEN,
                Json(json!({ "type": "error", "code": "ops_ip_forbidden" })),
            )
                .into_response());
        };
        if !crate::ops_allowlist::ip_allowed(client, &state.ops_allowlist) {
            return Err((
                StatusCode::FORBIDDEN,
                Json(json!({ "type": "error", "code": "ops_ip_forbidden" })),
            )
                .into_response());
        }
    }

    let Some(expected) = state.ops_token.as_deref() else {
        return Ok(());
    };
    let provided = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|value| {
            let (scheme, rest) = value.split_once(' ')?;
            if scheme.eq_ignore_ascii_case("bearer") {
                let rest = rest.trim();
                (!rest.is_empty()).then(|| rest.to_owned())
            } else {
                None
            }
        })
        .or_else(|| {
            ops_token_query
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        });
    if provided.as_deref() == Some(expected) {
        Ok(())
    } else {
        Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({ "type": "error", "code": "ops_unauthorized" })),
        )
            .into_response())
    }
}

pub(crate) fn lock_engine(state: &AppState) -> MutexGuard<'_, PaperEngine> {
    state
        .engine
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub(crate) fn resolve_account(state: &AppState, subject: &str) -> Result<AccountId, FanoutError> {
    state
        .accounts
        .get(subject)
        .copied()
        .ok_or(FanoutError::InvalidCredentials)
}

fn env_u64(key: &str) -> Option<u64> {
    std::env::var(key).ok()?.parse().ok()
}

fn parse_tokens(raw: Option<&str>) -> Vec<(String, String)> {
    raw.map(|s| {
        s.split(',')
            .filter_map(|entry| {
                let (token, subject) = entry.split_once(':')?;
                let token = token.trim();
                let subject = subject.trim();
                if token.is_empty() || subject.is_empty() {
                    return None;
                }
                Some((token.to_owned(), subject.to_owned()))
            })
            .collect()
    })
    .unwrap_or_default()
}

fn parse_clients(raw: Option<&str>) -> Vec<(String, String, String)> {
    raw.map(|s| {
        s.split(',')
            .filter_map(|entry| {
                let mut parts = entry.split(':');
                let id = parts.next()?.trim();
                let secret = parts.next()?.trim();
                let subject = parts.next()?.trim();
                if id.is_empty() || secret.is_empty() || subject.is_empty() {
                    return None;
                }
                Some((id.to_owned(), secret.to_owned(), subject.to_owned()))
            })
            .collect()
    })
    .unwrap_or_default()
}

fn parse_accounts(raw: Option<&str>) -> Vec<(String, u64)> {
    raw.map(|s| {
        s.split(',')
            .filter_map(|entry| {
                let (subject, raw_id) = entry.split_once(':')?;
                let subject = subject.trim();
                let raw_id = raw_id.trim().parse().ok()?;
                if subject.is_empty() {
                    return None;
                }
                Some((subject.to_owned(), raw_id))
            })
            .collect()
    })
    .unwrap_or_default()
}

fn parse_deposits(raw: Option<&str>) -> Vec<(u64, i64)> {
    raw.map(|s| {
        s.split(',')
            .filter_map(|entry| {
                let (account, major) = entry.split_once(':')?;
                let account = account.trim().parse().ok()?;
                let major = major.trim().parse().ok()?;
                Some((account, major))
            })
            .collect()
    })
    .unwrap_or_default()
}

fn parse_symbol_marks(raw: Option<&str>) -> Vec<(String, i64)> {
    raw.map(|s| {
        s.split(',')
            .filter_map(|entry| {
                let (symbol, scaled) = entry.split_once(':')?;
                let symbol = symbol.trim().to_owned();
                let scaled = scaled.trim().parse().ok()?;
                if symbol.is_empty() || scaled <= 0 {
                    return None;
                }
                Some((symbol, scaled))
            })
            .collect()
    })
    .unwrap_or_default()
}

fn parse_venue_kind(raw: Option<&str>) -> VenueKind {
    match raw.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
        Some("sandbox" | "sbx" | "broker") => VenueKind::Sandbox,
        Some("rest" | "http") => VenueKind::Rest,
        Some("licensed" | "fix" | "broker_sandbox") => VenueKind::Licensed,
        Some("alpaca" | "alpaca_paper") => VenueKind::Alpaca,
        _ => VenueKind::Sim,
    }
}

fn parse_csv_list(raw: Option<&str>) -> Vec<String> {
    let Some(raw) = raw.filter(|s| !s.is_empty()) else {
        return Vec::new();
    };
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Updates stored marks from a filled/working order's average or limit price.
pub(crate) fn record_fill_mark(state: &AppState, order: &shinrai_orders::Order) {
    let price = order.avg_px().unwrap_or_else(|| order.price());
    if order.cum_qty().lots() > 0 {
        state
            .marks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .set(order.instrument_id(), price);
        let mut engine = lock_engine(state);
        engine.set_mark(order.instrument_id(), price);
    }
}

/// Pushes gateway portfolio marks into the paper engine (risk path).
pub(crate) fn sync_marks_to_engine(state: &AppState) {
    let snap = state
        .marks
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .snapshot();
    let mut engine = lock_engine(state);
    engine.merge_marks(snap);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_accounts_and_deposits() {
        assert_eq!(
            parse_accounts(Some("alice:1,bob:2")),
            vec![("alice".into(), 1), ("bob".into(), 2)]
        );
        assert_eq!(
            parse_deposits(Some("1:10000,2:5000")),
            vec![(1, 10_000), (2, 5000)]
        );
    }
}
