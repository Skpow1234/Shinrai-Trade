//! Authoritative write-through of paper state to `shinrai-store`.

use shinrai_audit::AuditRecord;
use shinrai_orders::Order;
use shinrai_paper::PaperEngine;
use shinrai_store::{
    persist_trading_batch, DropCopyFillSnapshot, LedgerEntrySnapshot, OrderSnapshot,
    PaperPositionSnapshot, StoreError, StorePool, TradingBatch, VenueSessionCursorSnapshot,
};

/// Snapshot collected under the engine lock for async persistence.
#[derive(Debug, Clone)]
pub(crate) struct PersistBatch {
    pub(crate) order: Option<OrderSnapshot>,
    pub(crate) ledger: Vec<LedgerEntrySnapshot>,
    pub(crate) audit: Vec<AuditRecord>,
    pub(crate) positions: Vec<PaperPositionSnapshot>,
    pub(crate) drop_copy: Vec<DropCopyFillSnapshot>,
    pub(crate) venue_cursor: Option<VenueSessionCursorSnapshot>,
}

/// Collects order + full ledger journal + audit rows after `after_seq` + positions + drop-copy.
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
    let drop_copy = engine
        .durable_trade_execs()
        .iter()
        .map(|t| DropCopyFillSnapshot {
            order_id: t.order_id,
            exec_id: t.exec_id.as_str().to_owned(),
            qty: t.qty,
            price: t.price,
            session_n: t.session.n,
            seq: t.seq,
        })
        .collect();
    let (session, next_seq) = engine.applied_venue_cursor();
    let venue_cursor = Some(VenueSessionCursorSnapshot {
        applied_session_n: session.map(|s| s.n),
        next_expected_seq: next_seq,
    });
    PersistBatch {
        order: order.map(OrderSnapshot::from_order),
        ledger,
        audit,
        positions,
        drop_copy,
        venue_cursor,
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
        drop_copy: batch.drop_copy.clone(),
        venue_cursor: batch.venue_cursor,
    };
    persist_trading_batch(pool, &trading).await
}
