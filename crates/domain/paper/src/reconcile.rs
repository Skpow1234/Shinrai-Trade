//! OMS vs simulated-venue reconciliation.

use shinrai_orders::{OrderId, OrderStatus};

use crate::engine::PaperEngine;

/// Kind of reconciliation mismatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconciliationKind {
    /// OMS cumulative fill qty differs from the venue.
    FillQtyMismatch,
    /// OMS shows working but the venue has no record (after ack).
    OmsWorkingNotAtVenue,
    /// Venue has an order unknown to the OMS.
    VenueOrderUnknownToOms,
    /// Venue marked canceled while OMS is still working.
    VenueCanceledOmsWorking,
}

impl ReconciliationKind {
    /// Stable API code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::FillQtyMismatch => "fill_qty_mismatch",
            Self::OmsWorkingNotAtVenue => "oms_working_not_at_venue",
            Self::VenueOrderUnknownToOms => "venue_order_unknown_to_oms",
            Self::VenueCanceledOmsWorking => "venue_canceled_oms_working",
        }
    }
}

/// One reconciliation difference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconciliationMismatch {
    /// Mismatch category.
    pub kind: ReconciliationKind,
    /// Affected order.
    pub order_id: OrderId,
    /// Human-readable detail (no secrets).
    pub detail: String,
}

/// Result of comparing OMS state to the simulated venue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconciliationReport {
    /// True when `mismatches` is empty.
    pub ok: bool,
    /// Detected differences.
    pub mismatches: Vec<ReconciliationMismatch>,
}

impl PaperEngine {
    /// Compares OMS fill/working state to the simulated venue snapshot.
    #[must_use]
    pub fn reconcile(&self) -> ReconciliationReport {
        let mut mismatches = Vec::new();

        for order in self.orders().orders() {
            let id = order.id();
            let venue = self.sim().venue_order(id);
            let oms_cum = order.cum_qty().lots();

            if let Some(v) = venue {
                if v.cum_qty != oms_cum {
                    mismatches.push(ReconciliationMismatch {
                        kind: ReconciliationKind::FillQtyMismatch,
                        order_id: id,
                        detail: format!("oms_cum={oms_cum} venue_cum={}", v.cum_qty),
                    });
                }
                if v.canceled && order.status().is_working() {
                    mismatches.push(ReconciliationMismatch {
                        kind: ReconciliationKind::VenueCanceledOmsWorking,
                        order_id: id,
                        detail: format!("oms_status={}", order.status()),
                    });
                }
            } else if order.venue_order_id().is_some()
                && (order.status().is_working() || order.status() == OrderStatus::PendingCancel)
            {
                mismatches.push(ReconciliationMismatch {
                    kind: ReconciliationKind::OmsWorkingNotAtVenue,
                    order_id: id,
                    detail: format!("oms_status={}", order.status()),
                });
            }
        }

        for v in self.sim().venue_orders() {
            if self.orders().get(v.order_id).is_err() {
                mismatches.push(ReconciliationMismatch {
                    kind: ReconciliationKind::VenueOrderUnknownToOms,
                    order_id: v.order_id,
                    detail: "venue_inflight".into(),
                });
            }
        }

        ReconciliationReport {
            ok: mismatches.is_empty(),
            mismatches,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{PaperEngine, SubmitRequest};
    use shinrai_exchange_simulator::FaultConfig;
    use shinrai_instruments::{aapl, phase1_master, PriceTicks, QuantityLots};
    use shinrai_ledger::AccountId;
    use shinrai_money::{Currency, Money};
    use shinrai_orders::{ClientOrderId, Side};

    #[test]
    fn reconcile_ok_after_happy_path_fill() {
        let mut engine = PaperEngine::new(phase1_master(), FaultConfig::happy_path());
        let acc = AccountId::from_u64(1);
        engine
            .deposit(
                acc,
                Money::from_major(10_000, Currency::usd()).expect("d"),
                "dep",
            )
            .expect("dep");
        engine
            .submit(&SubmitRequest {
                account_id: acc,
                client_order_id: ClientOrderId::new("r1").expect("c"),
                instrument_id: aapl().id(),
                side: Side::Buy,
                qty: QuantityLots::from_lots(5),
                price: PriceTicks::from_scaled(10_000),
            })
            .expect("s");
        assert!(engine.reconcile().ok);
    }
}
