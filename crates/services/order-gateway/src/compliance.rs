//! KYC status gate, vendor hook, and dual-control approval helpers.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use shinrai_instruments::InstrumentId;
use shinrai_ledger::AccountId;
use tokio::runtime::{Builder, Handle, RuntimeFlavor};

/// KYC / trading eligibility for a subject.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KycStatus {
    /// May trade.
    Approved,
    /// Onboarding incomplete.
    Pending,
    /// Explicitly blocked.
    Rejected,
}

impl KycStatus {
    /// Parses `approved` / `pending` / `rejected` (case-insensitive).
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "approved" | "ok" | "pass" => Some(Self::Approved),
            "pending" | "review" => Some(Self::Pending),
            "rejected" | "denied" | "fail" => Some(Self::Rejected),
            _ => None,
        }
    }

    /// Stable API code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Approved => "approved",
            Self::Pending => "pending",
            Self::Rejected => "rejected",
        }
    }

    /// Whether trading is allowed.
    #[must_use]
    pub const fn allows_trading(self) -> bool {
        matches!(self, Self::Approved)
    }
}

/// Pluggable KYC lookup (env map or vendor HTTP).
pub trait KycProvider: Send + Sync {
    /// Resolves status for a subject id.
    fn status(&self, subject: &str) -> KycStatus;
}

/// In-memory KYC map (subject → status). Empty map = all approved (dev default).
#[derive(Debug, Clone, Default)]
pub struct KycRegistry {
    by_subject: HashMap<String, KycStatus>,
}

impl KycRegistry {
    /// Empty registry (fail-open: unknown subjects are approved).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Parses `subject:status,...` from `SHINRAI_OG_KYC_STATUS`.
    #[must_use]
    pub fn from_env_str(raw: Option<&str>) -> Self {
        let mut reg = Self::new();
        let Some(raw) = raw.filter(|s| !s.is_empty()) else {
            return reg;
        };
        for part in raw.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let Some((subj, status)) = part.split_once(':') else {
                continue;
            };
            if let Some(st) = KycStatus::parse(status) {
                reg.by_subject.insert(subj.trim().to_owned(), st);
            }
        }
        reg
    }

    /// Looks up status; unknown subjects are approved when the map is empty,
    /// otherwise pending (fail closed once any KYC rows are configured).
    #[must_use]
    pub fn status(&self, subject: &str) -> KycStatus {
        if let Some(st) = self.by_subject.get(subject) {
            return *st;
        }
        if self.by_subject.is_empty() {
            KycStatus::Approved
        } else {
            KycStatus::Pending
        }
    }

    /// Sets status (tests / ops).
    #[allow(dead_code)]
    pub fn set(&mut self, subject: impl Into<String>, status: KycStatus) {
        self.by_subject.insert(subject.into(), status);
    }
}

impl KycProvider for KycRegistry {
    fn status(&self, subject: &str) -> KycStatus {
        Self::status(self, subject)
    }
}

#[derive(Debug, Clone)]
struct CacheEntry {
    status: KycStatus,
    expires_at: Instant,
}

/// HTTP KYC vendor: `GET {base}/{subject}` expecting `{"status":"approved"}`.
///
/// Hardening: bearer token, short timeout, positive TTL cache, fail-closed on
/// transport / parse errors. Uses async `reqwest` driven via [`block_on_http`]
/// (same bridge pattern as venue HTTP).
#[derive(Debug, Clone)]
pub struct HttpKycVendor {
    base_url: String,
    bearer: Option<String>,
    cache_ttl: Duration,
    cache: Arc<Mutex<HashMap<String, CacheEntry>>>,
    client: reqwest::Client,
}

impl HttpKycVendor {
    /// Builds vendor from `SHINRAI_OG_KYC_VENDOR_URL`.
    ///
    /// Returns `None` when the URL is unset or the async HTTP client cannot be
    /// built (caller keeps the env-map registry).
    #[must_use]
    pub fn from_env() -> Option<Self> {
        let base_url = std::env::var("SHINRAI_OG_KYC_VENDOR_URL")
            .ok()
            .filter(|s| !s.is_empty())?;
        let bearer = std::env::var("SHINRAI_OG_KYC_VENDOR_TOKEN")
            .ok()
            .filter(|s| !s.is_empty());
        let timeout_ms = std::env::var("SHINRAI_OG_KYC_VENDOR_TIMEOUT_MS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(2_000_u64);
        let cache_ttl_secs = std::env::var("SHINRAI_OG_KYC_VENDOR_CACHE_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(60_u64);
        if let Some(v) = Self::new(
            base_url,
            bearer,
            Duration::from_millis(timeout_ms.max(100)),
            Duration::from_secs(cache_ttl_secs),
        ) {
            Some(v)
        } else {
            eprintln!(
                "shinrai-order-gateway: SHINRAI_OG_KYC_VENDOR_URL set but HTTP client \
                 failed to build; using env KYC map only"
            );
            None
        }
    }

    /// Constructs a vendor (tests / custom wiring).
    #[must_use]
    pub fn new(
        base_url: impl Into<String>,
        bearer: Option<String>,
        timeout: Duration,
        cache_ttl: Duration,
    ) -> Option<Self> {
        let client = reqwest::Client::builder().timeout(timeout).build().ok()?;
        Some(Self {
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            bearer,
            cache_ttl,
            cache: Arc::new(Mutex::new(HashMap::new())),
            client,
        })
    }

    fn cached(&self, subject: &str) -> Option<KycStatus> {
        let now = Instant::now();
        let guard = self
            .cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.get(subject).and_then(|e| {
            if e.expires_at > now {
                Some(e.status)
            } else {
                None
            }
        })
    }

    fn put_cache(&self, subject: &str, status: KycStatus) {
        if self.cache_ttl.is_zero() {
            return;
        }
        let mut guard = self
            .cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.insert(
            subject.to_owned(),
            CacheEntry {
                status,
                expires_at: Instant::now() + self.cache_ttl,
            },
        );
    }

    async fn fetch_status(&self, subject: &str) -> KycStatus {
        let encoded: String = subject
            .bytes()
            .map(|b| match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    (b as char).to_string()
                }
                _ => format!("%{b:02X}"),
            })
            .collect();
        let url = format!("{}/{}", self.base_url, encoded);
        let mut req = self.client.get(&url);
        if let Some(token) = &self.bearer {
            req = req.bearer_auth(token);
        }
        let Ok(resp) = req.send().await else {
            return KycStatus::Pending;
        };
        if !resp.status().is_success() {
            return KycStatus::Pending;
        }
        let Ok(body) = resp.json::<serde_json::Value>().await else {
            return KycStatus::Pending;
        };
        body.get("status")
            .and_then(|v| v.as_str())
            .and_then(KycStatus::parse)
            .unwrap_or(KycStatus::Pending)
    }
}

impl KycProvider for HttpKycVendor {
    fn status(&self, subject: &str) -> KycStatus {
        if let Some(st) = self.cached(subject) {
            return st;
        }
        let vendor = self.clone();
        let subject_owned = subject.to_owned();
        let status = block_on_http(async move { vendor.fetch_status(&subject_owned).await });
        self.put_cache(subject, status);
        status
    }
}

/// Drive async KYC HTTP from sync [`KycProvider::status`].
///
/// Multi-thread Tokio (order-gateway): [`tokio::task::block_in_place`].
/// Otherwise a dedicated current-thread runtime (tests / no Handle).
fn block_on_http<F, T>(fut: F) -> T
where
    F: Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    if let Ok(handle) = Handle::try_current() {
        if handle.runtime_flavor() == RuntimeFlavor::MultiThread {
            return tokio::task::block_in_place(|| handle.block_on(fut));
        }
    }
    std::thread::spawn(move || {
        Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime for KYC HTTP")
            .block_on(fut)
    })
    .join()
    .expect("KYC HTTP thread panicked")
}

/// Composite provider: vendor if configured, else registry.
#[derive(Clone)]
pub struct KycGate {
    inner: Arc<dyn KycProvider>,
}

impl KycGate {
    /// Env registry + optional HTTP vendor.
    #[must_use]
    pub fn from_env(raw: Option<&str>) -> Self {
        let registry = KycRegistry::from_env_str(raw);
        if let Some(vendor) = HttpKycVendor::from_env() {
            Self {
                inner: Arc::new(vendor),
            }
        } else {
            Self {
                inner: Arc::new(registry),
            }
        }
    }

    /// Status for subject.
    #[must_use]
    pub fn status(&self, subject: &str) -> KycStatus {
        self.inner.status(subject)
    }
}

impl std::fmt::Debug for KycGate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("KycGate(..)")
    }
}

/// Dual-control approval request for restricted instruments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalRequest {
    /// Request id.
    pub id: u64,
    /// Account.
    pub account_id: AccountId,
    /// Instrument.
    pub instrument_id: InstrumentId,
    /// Symbol display.
    pub symbol: String,
    /// Ops actor who requested.
    pub requested_by: String,
    /// Second ops actor who approved (None = pending).
    pub approved_by: Option<String>,
}

/// In-memory dual-control approval store.
#[derive(Debug, Default)]
pub struct ApprovalStore {
    next_id: u64,
    pending: HashMap<u64, ApprovalRequest>,
}

impl ApprovalStore {
    /// Creates empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces contents from durable snapshots (startup hydrate).
    pub fn restore_from(&mut self, rows: impl IntoIterator<Item = ApprovalRequest>) {
        self.pending.clear();
        self.next_id = 0;
        for req in rows {
            self.next_id = self.next_id.max(req.id);
            self.pending.insert(req.id, req);
        }
    }

    /// Requests an approval (first control). Uses `forced_id` when provided (DB-assigned).
    #[allow(dead_code)]
    pub fn request(
        &mut self,
        account_id: AccountId,
        instrument_id: InstrumentId,
        symbol: impl Into<String>,
        requested_by: impl Into<String>,
    ) -> ApprovalRequest {
        self.request_with_id(None, account_id, instrument_id, symbol, requested_by)
    }

    /// Like [`Self::request`] but pins the id (Postgres `RETURNING id`).
    pub fn request_with_id(
        &mut self,
        forced_id: Option<u64>,
        account_id: AccountId,
        instrument_id: InstrumentId,
        symbol: impl Into<String>,
        requested_by: impl Into<String>,
    ) -> ApprovalRequest {
        let id = if let Some(id) = forced_id {
            self.next_id = self.next_id.max(id);
            id
        } else {
            self.next_id = self.next_id.saturating_add(1);
            self.next_id
        };
        let req = ApprovalRequest {
            id,
            account_id,
            instrument_id,
            symbol: symbol.into(),
            requested_by: requested_by.into(),
            approved_by: None,
        };
        self.pending.insert(req.id, req.clone());
        req
    }

    /// Second-control approve; returns request when maker ≠ checker.
    pub fn approve(
        &mut self,
        id: u64,
        approved_by: impl Into<String>,
    ) -> Result<ApprovalRequest, &'static str> {
        let approved_by = approved_by.into();
        let Some(req) = self.pending.get_mut(&id) else {
            return Err("not_found");
        };
        if req.approved_by.is_some() {
            return Err("already_approved");
        }
        if req.requested_by == approved_by {
            return Err("dual_control_same_actor");
        }
        req.approved_by = Some(approved_by);
        Ok(req.clone())
    }

    /// Lists pending (unapproved) requests.
    #[must_use]
    pub fn pending(&self) -> Vec<ApprovalRequest> {
        self.pending
            .values()
            .filter(|r| r.approved_by.is_none())
            .cloned()
            .collect()
    }
}

/// Shared approval store handle.
pub type SharedApprovals = Arc<Mutex<ApprovalStore>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_registry_approves() {
        let reg = KycRegistry::new();
        assert_eq!(reg.status("alice"), KycStatus::Approved);
    }

    #[test]
    fn configured_unknown_is_pending() {
        let reg = KycRegistry::from_env_str(Some("alice:approved"));
        assert_eq!(reg.status("alice"), KycStatus::Approved);
        assert_eq!(reg.status("bob"), KycStatus::Pending);
    }

    #[test]
    fn dual_control_rejects_same_actor() {
        let mut store = ApprovalStore::new();
        let req = store.request(
            AccountId::from_u64(1),
            InstrumentId::from_u64(1),
            "AAPL",
            "ops-a",
        );
        assert_eq!(
            store.approve(req.id, "ops-a"),
            Err("dual_control_same_actor")
        );
        assert!(store.approve(req.id, "ops-b").is_ok());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn http_kyc_vendor_async_round_trip() {
        use axum::routing::get;
        use axum::{Json, Router};
        use serde_json::json;

        let app = Router::new().route(
            "/alice",
            get(|| async { Json(json!({ "status": "approved" })) }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve");
        });

        let vendor = HttpKycVendor::new(
            format!("http://{addr}"),
            Some("tok".into()),
            Duration::from_secs(2),
            Duration::from_secs(60),
        )
        .expect("client");
        assert_eq!(vendor.status("alice"), KycStatus::Approved);
        // Cached — second call must not need the server.
        assert_eq!(vendor.status("alice"), KycStatus::Approved);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn http_kyc_vendor_fail_closed_on_error() {
        use axum::routing::get;
        use axum::Router;

        let app = Router::new().route(
            "/bob",
            get(|| async { axum::http::StatusCode::BAD_GATEWAY }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve");
        });

        let vendor = HttpKycVendor::new(
            format!("http://{addr}"),
            None,
            Duration::from_secs(2),
            Duration::from_secs(0),
        )
        .expect("client");
        assert_eq!(vendor.status("bob"), KycStatus::Pending);
    }

    #[test]
    fn block_on_http_without_runtime() {
        let n = block_on_http(async { 3_u32.saturating_add(1) });
        assert_eq!(n, 4);
    }
}
