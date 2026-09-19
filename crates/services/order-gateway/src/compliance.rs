//! KYC status gate, vendor hook, and dual-control approval helpers.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use shinrai_instruments::InstrumentId;
use shinrai_ledger::AccountId;

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

/// HTTP KYC vendor stub: `GET {base}/{subject}` expecting `{"status":"approved"}`.
///
/// When unreachable or misconfigured, fails closed to [`KycStatus::Pending`].
#[derive(Debug, Clone)]
pub struct HttpKycVendor {
    base_url: String,
    fallback: KycRegistry,
}

impl HttpKycVendor {
    /// Builds vendor from `SHINRAI_OG_KYC_VENDOR_URL` with env-map fallback.
    #[must_use]
    pub fn from_env(fallback: KycRegistry) -> Option<Self> {
        let base_url = std::env::var("SHINRAI_OG_KYC_VENDOR_URL")
            .ok()
            .filter(|s| !s.is_empty())?;
        Some(Self {
            base_url: base_url.trim_end_matches('/').to_owned(),
            fallback,
        })
    }
}

impl KycProvider for HttpKycVendor {
    fn status(&self, subject: &str) -> KycStatus {
        let url = format!("{}/{}", self.base_url, subject);
        let Ok(client) = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(2))
            .build()
        else {
            return self.fallback.status(subject);
        };
        let Ok(resp) = client.get(&url).send() else {
            return KycStatus::Pending;
        };
        if !resp.status().is_success() {
            return KycStatus::Pending;
        }
        let Ok(body) = resp.json::<serde_json::Value>() else {
            return KycStatus::Pending;
        };
        body.get("status")
            .and_then(|v| v.as_str())
            .and_then(KycStatus::parse)
            .unwrap_or(KycStatus::Pending)
    }
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
        if let Some(vendor) = HttpKycVendor::from_env(registry.clone()) {
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

    /// Requests an approval (first control).
    pub fn request(
        &mut self,
        account_id: AccountId,
        instrument_id: InstrumentId,
        symbol: impl Into<String>,
        requested_by: impl Into<String>,
    ) -> ApprovalRequest {
        self.next_id = self.next_id.saturating_add(1);
        let req = ApprovalRequest {
            id: self.next_id,
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
}
