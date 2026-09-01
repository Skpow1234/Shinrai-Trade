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
use shinrai_instruments::{phase1_master, InstrumentMaster};
use shinrai_ledger::AccountId;
use shinrai_md_fanout::{FanoutError, SubjectId, TokenAuth, TokenTtl};
use shinrai_money::{Currency, Money};
use shinrai_paper::PaperEngine;
use shinrai_portfolio::MarkStore;
use shinrai_risk::{RiskEngine, RiskLimits};

/// Coarse gateway counters for local ops (not billing-grade).
#[derive(Debug, Default)]
#[allow(clippy::struct_field_names)]
pub struct GatewayMetrics {
    orders_submitted: AtomicU64,
    orders_accepted: AtomicU64,
    orders_risk_rejected: AtomicU64,
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

    /// JSON snapshot for `GET /v1/metrics`.
    #[must_use]
    pub fn snapshot(&self) -> Value {
        json!({
            "orders_submitted": self.orders_submitted.load(Ordering::Relaxed),
            "orders_accepted": self.orders_accepted.load(Ordering::Relaxed),
            "orders_risk_rejected": self.orders_risk_rejected.load(Ordering::Relaxed),
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
            .finish()
    }
}

impl AppState {
    /// Builds state from config.
    #[must_use]
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
        let mut engine = PaperEngine::with_risk(
            master.clone(),
            FaultConfig::happy_path(),
            RiskEngine::new(RiskLimits::demo()),
        );

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
                    marks.set(id, shinrai_instruments::PriceTicks::from_scaled(*scaled));
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
        }
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
        .route("/v1/portfolio", get(crate::portfolio_http::get_portfolio))
        .route("/v1/audit", get(crate::portfolio_http::get_audit))
        .route(
            "/v1/reconciliation",
            get(crate::portfolio_http::get_reconciliation),
        )
        .route("/v1/metrics", get(crate::portfolio_http::get_metrics))
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

/// Updates stored marks from a filled/working order's average or limit price.
pub(crate) fn record_fill_mark(state: &AppState, order: &shinrai_orders::Order) {
    let price = order.avg_px().unwrap_or_else(|| order.price());
    if order.cum_qty().lots() > 0 {
        state
            .marks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .set(order.instrument_id(), price);
    }
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
