//! Authoritative write-through of paper state to `shinrai-store`.

use shinrai_audit::AuditRecord;
use shinrai_orders::Order;
use shinrai_paper::PaperEngine;
use shinrai_store::{
    persist_trading_batch, LedgerEntrySnapshot, OrderSnapshot, PaperPositionSnapshot, StoreError,
    StorePool, TradingBatch,
};

/// Snapshot collected under the engine lock for async persistence.
#[derive(Debug, Clone)]
pub(crate) struct PersistBatch {
    pub(crate) order: Option<OrderSnapshot>,
    pub(crate) ledger: Vec<LedgerEntrySnapshot>,
    pub(crate) audit: Vec<AuditRecord>,
    pub(crate) positions: Vec<PaperPositionSnapshot>,
}

/// Collects order + full ledger journal + audit rows after `after_seq` + positions.
pub(crate) fn collect_batch(
    engine: &PaperEngine,
    order: Option<&Order>,
    after_audit_seq: u64,
) -> PersistBatch {
    let ledger = engine
        .book()
        .journal()
        .entries()
        .map(|(_, entry)| LedgerEntrySnapshot::from_balanced(entry))
        .collect();
    let audit = engine
        .audit()
        .records()
        .filter(|r| r.seq() > after_audit_seq)
        .cloned()
        .collect();
    let positions = engine
        .book()
        .positions_iter()
        .map(
            |(account_id, instrument_id, lots, reserved_lots)| PaperPositionSnapshot {
                account_id,
                instrument_id,
                lots,
                reserved_lots,
            },
        )
        .collect();
    PersistBatch {
        order: order.map(OrderSnapshot::from_order),
        ledger,
        audit,
        positions,
    }
}

/// Writes a batch in one transaction. Required when Postgres is configured.
///
/// # Errors
///
/// Returns store errors (caller must not ack the client).
pub(crate) async fn write_batch(pool: &StorePool, batch: &PersistBatch) -> Result<(), StoreError> {
    let trading = TradingBatch {
        order: batch.order.clone(),
        ledger: batch.ledger.clone(),
        audit: batch.audit.clone(),
        positions: batch.positions.clone(),
    };
    persist_trading_batch(pool, &trading).await
}
