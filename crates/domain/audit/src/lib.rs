//! Audit event kinds on the trading path.

use core::fmt;

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
}

impl fmt::Display for AuditKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// One immutable audit row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditRecord {
    seq: u64,
    at: u64,
    account_id: Option<AccountId>,
    order_id: Option<OrderId>,
    kind: AuditKind,
}

impl AuditRecord {
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
}

/// Append-only in-memory audit log (rebuildable from durable events later).
#[derive(Debug, Default, Clone)]
pub struct AuditLog {
    next_seq: u64,
    records: Vec<AuditRecord>,
}

impl AuditLog {
    /// Empty log.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
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
        let row = AuditRecord {
            seq: self.next_seq,
            at,
            account_id,
            order_id,
            kind,
        };
        self.records.push(row);
        // SAFETY: we just pushed.
        self.records.last().expect("just pushed")
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
    }
}
