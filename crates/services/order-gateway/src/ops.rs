//! Operational snapshots: order status counts and stuck pending detection.

use std::collections::HashMap;

use serde_json::{json, Value};
use shinrai_orders::{OrderId, OrderStatus};
use shinrai_paper::PaperEngine;

/// Default stuck age for paper/sim (seconds of logical clock).
pub const DEFAULT_STUCK_AGE_SECS: u64 = 5;

/// Statuses that should not linger without venue progress.
#[must_use]
pub const fn is_pending_ops_status(status: OrderStatus) -> bool {
    matches!(
        status,
        OrderStatus::PendingNew | OrderStatus::PendingCancel | OrderStatus::PendingReplace
    )
}

/// One order considered stuck by age.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StuckOrder {
    /// Internal order id.
    pub order_id: OrderId,
    /// Current OMS status.
    pub status: OrderStatus,
    /// Age in logical seconds (`now - last_seen_at`).
    pub age_secs: u64,
    /// Last audit timestamp used for age (unix seconds).
    pub last_at: u64,
}

/// Builds OMS status histogram + stuck list for ops endpoints.
#[must_use]
pub fn ops_snapshot(engine: &PaperEngine, now: u64, stuck_age_secs: u64) -> Value {
    let mut by_status: HashMap<&'static str, u64> = HashMap::new();
    for status in OrderStatus::all() {
        by_status.insert(status_label(status), 0);
    }
    let mut working = 0_u64;
    let mut terminal = 0_u64;
    let mut pending = 0_u64;

    for order in engine.orders().orders() {
        let label = status_label(order.status());
        *by_status.entry(label).or_insert(0) += 1;
        if order.status().is_working() {
            working += 1;
        }
        if order.status().is_terminal() {
            terminal += 1;
        }
        if is_pending_ops_status(order.status()) {
            pending += 1;
        }
    }

    let stuck = find_stuck_orders(engine, now, stuck_age_secs);
    let recon = engine.reconcile();

    json!({
        "orders_total": engine.orders().len(),
        "orders_working": working,
        "orders_terminal": terminal,
        "orders_pending": pending,
        "orders_by_status": by_status,
        "stuck_orders": stuck.iter().map(|s| json!({
            "order_id": s.order_id.get(),
            "status": s.status.to_string(),
            "age_secs": s.age_secs,
            "last_at": s.last_at,
        })).collect::<Vec<_>>(),
        "stuck_count": stuck.len(),
        "stuck_age_secs": stuck_age_secs,
        "ledger_entries": engine.book().journal().len(),
        "ledger_trial_balance_ok": engine.book().journal().trial_balance_ok(),
        "audit_records": engine.audit().len(),
        "reconciliation_ok": recon.ok,
        "reconciliation_mismatches": recon.mismatches.len(),
    })
}

/// Finds pending OMS orders older than `max_age_secs` (logical clock).
#[must_use]
pub fn find_stuck_orders(engine: &PaperEngine, now: u64, max_age_secs: u64) -> Vec<StuckOrder> {
    let mut last_at: HashMap<OrderId, u64> = HashMap::new();
    for record in engine.audit().records() {
        if let Some(oid) = record.order_id() {
            let slot = last_at.entry(oid).or_insert(0);
            *slot = (*slot).max(record.at());
        }
    }

    let mut stuck = Vec::new();
    for order in engine.orders().orders() {
        if !is_pending_ops_status(order.status()) {
            continue;
        }
        let at = last_at.get(&order.id()).copied().unwrap_or(0);
        let age = now.saturating_sub(at);
        if age >= max_age_secs {
            stuck.push(StuckOrder {
                order_id: order.id(),
                status: order.status(),
                age_secs: age,
                last_at: at,
            });
        }
    }
    stuck.sort_by_key(|s| std::cmp::Reverse(s.age_secs));
    stuck
}

const fn status_label(status: OrderStatus) -> &'static str {
    match status {
        OrderStatus::PendingNew => "PendingNew",
        OrderStatus::New => "New",
        OrderStatus::PartiallyFilled => "PartiallyFilled",
        OrderStatus::Filled => "Filled",
        OrderStatus::PendingCancel => "PendingCancel",
        OrderStatus::Canceled => "Canceled",
        OrderStatus::PendingReplace => "PendingReplace",
        OrderStatus::Rejected => "Rejected",
        OrderStatus::Expired => "Expired",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shinrai_exchange_simulator::FaultConfig;
    use shinrai_instruments::{phase1_master, PriceTicks, QuantityLots};
    use shinrai_ledger::AccountId;
    use shinrai_orders::{ClientOrderId, Order, OrderId, OrderType, Side};

    #[test]
    fn pending_new_without_progress_is_stuck() {
        let mut engine = PaperEngine::new(phase1_master(), FaultConfig::happy_path());
        let order = Order::restore(
            OrderId::from_u64(7),
            AccountId::from_u64(1),
            ClientOrderId::new("stuck-1").expect("c"),
            shinrai_instruments::InstrumentId::from_u64(1),
            Side::Buy,
            OrderType::Limit,
            OrderStatus::PendingNew,
            QuantityLots::from_lots(1),
            PriceTicks::from_scaled(100),
            QuantityLots::from_lots(0),
            QuantityLots::from_lots(1),
            None,
            None,
            None,
            Vec::new(),
        )
        .expect("order");
        engine
            .hydrate(Vec::new(), vec![order], Vec::new(), Vec::new())
            .expect("hydrate");
        let stuck = find_stuck_orders(&engine, 100, 5);
        assert_eq!(stuck.len(), 1);
        assert_eq!(stuck[0].order_id.get(), 7);
        assert_eq!(stuck[0].age_secs, 100);
    }
}
