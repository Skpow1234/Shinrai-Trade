//! Load durable state from `shinrai-store` into `PaperEngine`.

use shinrai_audit::AuditRecord;
use shinrai_execution::{SessionId, VenueTradeSnapshot};
use shinrai_instruments::InstrumentId;
use shinrai_ledger::{AccountId, BalancedEntry};
use shinrai_orders::{ExecId, Order, Side};
use shinrai_paper::PaperEngine;
use shinrai_store::{
    list_drop_copy_fills, list_ledger_entries, list_orders, list_paper_positions, load_audit_after,
    DropCopyFillSnapshot, StoreError, StorePool,
};

/// Loaded snapshots ready to apply under the engine lock.
pub(crate) struct HydratePayload {
    ledger: Vec<BalancedEntry>,
    orders: Vec<Order>,
    audit: Vec<AuditRecord>,
    positions: Vec<(AccountId, InstrumentId, i64, i64)>,
    drop_copy: Vec<DropCopyFillSnapshot>,
    pub(crate) max_audit_seq: u64,
}

/// Loads hydrate payload from Postgres (no engine lock held).
///
/// # Errors
///
/// Returns store or decode errors.
pub(crate) async fn load_hydrate_payload(pool: &StorePool) -> Result<HydratePayload, StoreError> {
    let ledger_snaps = list_ledger_entries(pool).await?;
    let mut ledger = Vec::with_capacity(ledger_snaps.len());
    for snap in ledger_snaps {
        ledger.push(snap.try_into_balanced()?);
    }

    let order_snaps = list_orders(pool).await?;
    let mut orders = Vec::with_capacity(order_snaps.len());
    for snap in &order_snaps {
        orders.push(snap.to_order().map_err(|e| StoreError::InvalidStored {
            field: "order",
            value: e.to_string(),
        })?);
    }

    let mut positions: Vec<_> = list_paper_positions(pool)
        .await?
        .into_iter()
        .map(|p| (p.account_id, p.instrument_id, p.lots, p.reserved_lots))
        .collect();

    if positions.is_empty() {
        positions = derive_positions_from_orders(&orders);
    }

    let mut audit = Vec::new();
    let mut after = 0_u64;
    loop {
        let page = load_audit_after(pool, after, 500).await?;
        if page.is_empty() {
            break;
        }
        after = page.last().map_or(after, AuditRecord::seq);
        audit.extend(page);
    }
    let max_audit_seq = audit.iter().map(AuditRecord::seq).max().unwrap_or(0);

    let drop_copy = list_drop_copy_fills(pool).await?;
    // Venue session cursor is persisted for ops forensics but not reapplied on
    // hydrate: in-process venues restart a fresh outbound journal after process
    // restart, and restoring the old consumer cursor drops cancel/fill acks.

    Ok(HydratePayload {
        ledger,
        orders,
        audit,
        positions,
        drop_copy,
        max_audit_seq,
    })
}

/// Applies a previously loaded payload under the engine mutex (sync).
///
/// # Errors
///
/// Returns domain hydrate errors mapped to [`StoreError`].
pub(crate) fn apply_hydrate(
    engine: &mut PaperEngine,
    payload: HydratePayload,
) -> Result<u64, StoreError> {
    let max = payload.max_audit_seq;
    engine
        .hydrate(
            payload.ledger,
            payload.orders,
            payload.audit,
            payload.positions,
        )
        .map_err(|e| StoreError::InvalidStored {
            field: "hydrate",
            value: e.to_string(),
        })?;

    if !payload.drop_copy.is_empty() {
        let mut trades = Vec::with_capacity(payload.drop_copy.len());
        for snap in payload.drop_copy {
            let exec_id =
                ExecId::new(snap.exec_id.clone()).map_err(|e| StoreError::InvalidStored {
                    field: "drop_copy.exec_id",
                    value: e.to_string(),
                })?;
            trades.push(VenueTradeSnapshot {
                order_id: snap.order_id,
                exec_id,
                qty: snap.qty,
                price: snap.price,
                session: SessionId::new(snap.session_n),
                seq: snap.seq,
            });
        }
        engine.restore_durable_trades(trades);
    }

    Ok(max)
}

fn derive_positions_from_orders(orders: &[Order]) -> Vec<(AccountId, InstrumentId, i64, i64)> {
    use std::collections::HashMap;
    let mut map: HashMap<(AccountId, InstrumentId), i64> = HashMap::new();
    for order in orders {
        let lots = order.cum_qty().lots();
        if lots == 0 {
            continue;
        }
        let delta = match order.side() {
            Side::Buy => lots,
            Side::Sell => -lots,
        };
        *map.entry((order.account_id(), order.instrument_id()))
            .or_insert(0) += delta;
    }
    map.into_iter()
        .filter(|(_, lots)| *lots != 0)
        .map(|((acc, inst), lots)| (acc, inst, lots, 0))
        .collect()
}
