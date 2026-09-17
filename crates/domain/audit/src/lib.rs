//! Append-only audit trail with optional hash chain and correlation ids.

use core::fmt;

use sha2::{Digest, Sha256};
use shinrai_ledger::AccountId;
use shinrai_orders::OrderId;

/// Stable category for filtering and metrics.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AuditKind {
    /// Client or gateway requested order submit.
    OrderSubmitRequested,
    /// Pre-trade risk rejected before OMS mutation.
    RiskRejected {
        /// Stable rejection code.
        code: String,
    },
    /// OMS created a new order.
    OrderCreated,
    /// Idempotent duplicate client order id.
    OrderDuplicate,
    /// OMS applied a domain event.
    OrderEventApplied {
        /// Resulting status label.
        status: String,
    },
    /// Cash reserved for a working order.
    LedgerReserved,
    /// Buy fill settled on the ledger.
    LedgerSettled,
    /// Unused reservation released.
    LedgerReleased,
    /// Order forwarded to the simulated venue.
    VenueSubmitted,
    /// Execution report drained from the venue.
    VenueReport {
        /// Report label (`new`, `trade`, `canceled`, etc.).
        exec_type: String,
    },
}

impl AuditKind {
    /// Short stable name for APIs.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::OrderSubmitRequested => "order_submit_requested",
            Self::RiskRejected { .. } => "risk_rejected",
            Self::OrderCreated => "order_created",
            Self::OrderDuplicate => "order_duplicate",
            Self::OrderEventApplied { .. } => "order_event_applied",
            Self::LedgerReserved => "ledger_reserved",
            Self::LedgerSettled => "ledger_settled",
            Self::LedgerReleased => "ledger_released",
            Self::VenueSubmitted => "venue_submitted",
            Self::VenueReport { .. } => "venue_report",
        }
    }

    /// Detail payload for hashing / persistence.
    #[must_use]
    pub fn detail(&self) -> Option<&str> {
        match self {
            Self::RiskRejected { code } => Some(code.as_str()),
            Self::OrderEventApplied { status } => Some(status.as_str()),
            Self::VenueReport { exec_type } => Some(exec_type.as_str()),
            _ => None,
        }
    }
}

impl fmt::Display for AuditKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Genesis previous-hash for the first audit record.
pub const GENESIS_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// One immutable audit row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditRecord {
    seq: u64,
    at: u64,
    account_id: Option<AccountId>,
    order_id: Option<OrderId>,
    kind: AuditKind,
    correlation_id: Option<String>,
    prev_hash: String,
    content_hash: String,
}

impl AuditRecord {
    /// Reconstructs a record (e.g. from durable storage).
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn from_parts(
        seq: u64,
        at: u64,
        account_id: Option<AccountId>,
        order_id: Option<OrderId>,
        kind: AuditKind,
        correlation_id: Option<String>,
        prev_hash: String,
        content_hash: String,
    ) -> Self {
        Self {
            seq,
            at,
            account_id,
            order_id,
            kind,
            correlation_id,
            prev_hash,
            content_hash,
        }
    }

    /// Legacy reconstruct without hashes (hash fields empty; not chain-verifiable).
    #[must_use]
    pub fn from_parts_legacy(
        seq: u64,
        at: u64,
        account_id: Option<AccountId>,
        order_id: Option<OrderId>,
        kind: AuditKind,
    ) -> Self {
        Self::from_parts(
            seq,
            at,
            account_id,
            order_id,
            kind,
            None,
            String::new(),
            String::new(),
        )
    }

    /// Monotonic sequence (1-based).
    #[must_use]
    pub const fn seq(&self) -> u64 {
        self.seq
    }

    /// Logical timestamp (unix seconds at the gateway edge).
    #[must_use]
    pub const fn at(&self) -> u64 {
        self.at
    }

    /// Account when known.
    #[must_use]
    pub const fn account_id(&self) -> Option<AccountId> {
        self.account_id
    }

    /// Order when known.
    #[must_use]
    pub const fn order_id(&self) -> Option<OrderId> {
        self.order_id
    }

    /// Event kind.
    #[must_use]
    pub fn kind(&self) -> &AuditKind {
        &self.kind
    }

    /// Correlation id spanning the order path.
    #[must_use]
    pub fn correlation_id(&self) -> Option<&str> {
        self.correlation_id.as_deref()
    }

    /// Previous record content hash (or genesis).
    #[must_use]
    pub fn prev_hash(&self) -> &str {
        &self.prev_hash
    }

    /// Hash of this record's content + prev hash.
    #[must_use]
    pub fn content_hash(&self) -> &str {
        &self.content_hash
    }
}

/// Computes the content hash for an audit row.
#[must_use]
pub fn compute_content_hash(
    seq: u64,
    at: u64,
    account_id: Option<AccountId>,
    order_id: Option<OrderId>,
    kind: &AuditKind,
    correlation_id: Option<&str>,
    prev_hash: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(seq.to_be_bytes());
    hasher.update(at.to_be_bytes());
    hasher.update(account_id.map_or([0; 8], |a| a.get().to_be_bytes()));
    hasher.update(order_id.map_or([0; 8], |o| o.get().to_be_bytes()));
    hasher.update(kind.name().as_bytes());
    if let Some(d) = kind.detail() {
        hasher.update(d.as_bytes());
    }
    if let Some(c) = correlation_id {
        hasher.update(c.as_bytes());
    }
    hasher.update(prev_hash.as_bytes());
    hex_encode(&hasher.finalize())
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use fmt::Write as _;
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// Append-only in-memory audit log (rebuildable from durable events later).
#[derive(Debug, Clone)]
pub struct AuditLog {
    next_seq: u64,
    records: Vec<AuditRecord>,
    last_hash: String,
    correlation_id: Option<String>,
}

impl Default for AuditLog {
    fn default() -> Self {
        Self::new()
    }
}

impl AuditLog {
    /// Empty log.
    #[must_use]
    pub fn new() -> Self {
        Self {
            next_seq: 0,
            records: Vec::new(),
            last_hash: GENESIS_HASH.to_owned(),
            correlation_id: None,
        }
    }

    /// Sets the active correlation id for subsequent [`Self::record`] calls.
    pub fn set_correlation_id(&mut self, id: Option<String>) {
        self.correlation_id = id;
    }

    /// Active correlation id.
    #[must_use]
    pub fn correlation_id(&self) -> Option<&str> {
        self.correlation_id.as_deref()
    }

    /// Number of records.
    #[must_use]
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Returns true if empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Appends a record at logical time `at`.
    pub fn record(
        &mut self,
        at: u64,
        account_id: Option<AccountId>,
        order_id: Option<OrderId>,
        kind: AuditKind,
    ) -> &AuditRecord {
        self.next_seq = self.next_seq.saturating_add(1);
        let prev_hash = self.last_hash.clone();
        let correlation_id = self.correlation_id.clone();
        let content_hash = compute_content_hash(
            self.next_seq,
            at,
            account_id,
            order_id,
            &kind,
            correlation_id.as_deref(),
            &prev_hash,
        );
        self.last_hash.clone_from(&content_hash);
        let row = AuditRecord {
            seq: self.next_seq,
            at,
            account_id,
            order_id,
            kind,
            correlation_id,
            prev_hash,
            content_hash,
        };
        self.records.push(row);
        self.records.last().expect("just pushed")
    }

    /// Verifies the hash chain from genesis.
    #[must_use]
    pub fn verify_chain(&self) -> bool {
        let mut prev = GENESIS_HASH.to_owned();
        for row in &self.records {
            if row.prev_hash != prev {
                return false;
            }
            if row.content_hash.is_empty() {
                return false;
            }
            let expected = compute_content_hash(
                row.seq,
                row.at,
                row.account_id,
                row.order_id,
                &row.kind,
                row.correlation_id.as_deref(),
                &row.prev_hash,
            );
            if expected != row.content_hash {
                return false;
            }
            prev.clone_from(&row.content_hash);
        }
        true
    }

    /// All records in append order.
    pub fn records(&self) -> impl Iterator<Item = &AuditRecord> {
        self.records.iter()
    }

    /// Records for one account.
    pub fn for_account(&self, account: AccountId) -> impl Iterator<Item = &AuditRecord> {
        self.records
            .iter()
            .filter(move |r| r.account_id == Some(account))
    }

    /// Page by sequence (`after_seq` exclusive); newest last in the slice.
    #[must_use]
    pub fn page_after(&self, after_seq: u64, limit: usize) -> Vec<&AuditRecord> {
        self.records
            .iter()
            .filter(|r| r.seq() > after_seq)
            .take(limit)
            .collect()
    }

    /// Replaces the log with durable records (startup replay).
    ///
    /// Sets `next_seq` to the maximum restored sequence so new records continue.
    pub fn restore(&mut self, records: Vec<AuditRecord>) {
        let max = records.iter().map(AuditRecord::seq).max().unwrap_or(0);
        self.next_seq = max;
        self.last_hash = match records.last() {
            Some(r) if !r.content_hash.is_empty() => r.content_hash.clone(),
            _ => GENESIS_HASH.to_owned(),
        };
        self.records = records;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monotonic_seq_and_page() {
        let mut log = AuditLog::new();
        log.record(
            1,
            Some(AccountId::from_u64(1)),
            None,
            AuditKind::OrderSubmitRequested,
        );
        log.record(
            2,
            Some(AccountId::from_u64(1)),
            Some(OrderId::from_u64(9)),
            AuditKind::OrderCreated,
        );
        assert_eq!(log.len(), 2);
        assert_eq!(log.page_after(0, 10).len(), 2);
        assert_eq!(log.page_after(1, 10).len(), 1);
        assert!(log.verify_chain());
    }

    #[test]
    fn correlation_and_tamper_detection() {
        let mut log = AuditLog::new();
        log.set_correlation_id(Some("corr-1".into()));
        log.record(1, None, None, AuditKind::OrderSubmitRequested);
        assert_eq!(
            log.records().next().unwrap().correlation_id(),
            Some("corr-1")
        );
        assert!(log.verify_chain());
        log.records[0].content_hash = "deadbeef".into();
        assert!(!log.verify_chain());
    }
}
