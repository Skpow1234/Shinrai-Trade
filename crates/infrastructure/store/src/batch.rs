//! Single-transaction persistence of a trading mutation batch.

use sqlx::PgPool;

use shinrai_audit::AuditRecord;

use crate::audit::insert_audit_record_tx;
use crate::drop_copy::{upsert_drop_copy_fill_tx, DropCopyFillSnapshot};
use crate::error::StoreError;
use crate::ledger::{insert_ledger_entry_tx, LedgerEntrySnapshot};
use crate::orders::{upsert_order_tx, OrderSnapshot};
use crate::outbox;
use crate::positions::{upsert_paper_position_tx, PaperPositionSnapshot};
use crate::venue_session::{upsert_venue_session_cursor_tx, VenueSessionCursorSnapshot};

/// Order + ledger + audit + positions + drop-copy + venue cursor written atomically.
#[derive(Debug, Clone)]
pub struct TradingBatch {
    /// Optional order upsert.
    pub order: Option<OrderSnapshot>,
    /// Ledger entries (idempotent on key).
    pub ledger: Vec<LedgerEntrySnapshot>,
    /// New audit rows (`ON CONFLICT DO NOTHING`).
    pub audit: Vec<AuditRecord>,
    /// Paper positions upserts.
    pub positions: Vec<PaperPositionSnapshot>,
    /// Durable Trade drop-copy fills.
    pub drop_copy: Vec<DropCopyFillSnapshot>,
    /// Consumer venue session cursor.
    pub venue_cursor: Option<VenueSessionCursorSnapshot>,
}

/// Persists an entire trading batch in one Postgres transaction.
///
/// Ledger inserts that create a new row also enqueue `ledger.posted` outbox events
/// in the same transaction.
///
/// # Errors
///
/// Returns sqlx / store errors; the transaction is rolled back on failure.
pub async fn persist_trading_batch(pool: &PgPool, batch: &TradingBatch) -> Result<(), StoreError> {
    let mut tx = pool.begin().await?;

    if let Some(order) = &batch.order {
        upsert_order_tx(&mut tx, order).await?;
    }

    for entry in &batch.ledger {
        let payload = serde_json::json!({
            "idempotency_key": entry.idempotency_key,
            "kind": "ledger_posted",
        });
        insert_ledger_entry_tx(&mut tx, entry, Some("ledger.posted"), Some(payload)).await?;
    }

    for record in &batch.audit {
        insert_audit_record_tx(&mut tx, record).await?;
    }

    for pos in &batch.positions {
        upsert_paper_position_tx(&mut tx, pos).await?;
    }

    for fill in &batch.drop_copy {
        upsert_drop_copy_fill_tx(&mut tx, fill).await?;
    }

    if let Some(cursor) = &batch.venue_cursor {
        upsert_venue_session_cursor_tx(&mut tx, cursor).await?;
    }

    // Also emit an order lifecycle outbox when an order was upserted.
    if let Some(order) = &batch.order {
        let payload = serde_json::json!({
            "order_id": order.id.get(),
            "account_id": order.account_id.get(),
            "client_order_id": order.client_order_id.as_str(),
            "status": order.status.as_str(),
            "kind": "order.upserted",
        });
        outbox::insert_outbox_tx(&mut tx, "order.upserted", &payload).await?;
    }

    tx.commit().await?;
    Ok(())
}
