//! Dual-write in-memory paper state to `shinrai-store` when Postgres is configured.

use shinrai_audit::AuditRecord;
use shinrai_orders::Order;
use shinrai_paper::PaperEngine;
use shinrai_store::{
    insert_audit_record, insert_ledger_entry, upsert_order, LedgerEntrySnapshot, OrderSnapshot,
    StoreError, StorePool,
};

/// Snapshot collected under the engine lock for async persistence.
#[derive(Debug, Clone)]
pub(crate) struct PersistBatch {
    pub(crate) order: Option<OrderSnapshot>,
    pub(crate) ledger: Vec<LedgerEntrySnapshot>,
    pub(crate) audit: Vec<AuditRecord>,
}

/// Collects order + full ledger journal + audit rows after `after_seq`.
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
    PersistBatch {
        order: order.map(OrderSnapshot::from_order),
        ledger,
        audit,
    }
}

/// Writes a batch. Ledger inserts are idempotent; audit uses `ON CONFLICT DO NOTHING`.
///
/// # Errors
///
/// Returns the first store error.
pub(crate) async fn write_batch(pool: &StorePool, batch: &PersistBatch) -> Result<(), StoreError> {
    if let Some(order) = &batch.order {
        upsert_order(pool, order).await?;
    }
    for entry in &batch.ledger {
        let payload = serde_json::json!({
            "idempotency_key": entry.idempotency_key,
            "kind": "ledger_posted",
        });
        insert_ledger_entry(pool, entry, Some("ledger.posted"), Some(payload)).await?;
    }
    for record in &batch.audit {
        insert_audit_record(pool, record).await?;
    }
    Ok(())
}
