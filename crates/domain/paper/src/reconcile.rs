//! OMS vs venue drop-copy / ledger reconciliation.

use std::collections::HashSet;

use shinrai_instruments::InstrumentId;
use shinrai_ledger::AccountId;
use shinrai_money::Currency;
use shinrai_orders::{ExecId, OrderId, OrderStatus};

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
    /// Venue Trade exec id not present in OMS `seen_execs`.
    VenueExecMissingInOms,
    /// OMS `seen_execs` entry missing from current-session venue Trade journal.
    OmsExecMissingAtVenue,
    /// OMS `seen_execs` entry missing ledger key `fill:{exec_id}`.
    OmsExecMissingInLedger,
    /// Ledger `fill:{exec_id}` with no matching OMS `seen_execs`.
    LedgerFillWithoutOmsExec,
    /// Internal cash (available+reserved) differs from broker EOD cash.
    CashMismatch,
    /// Internal position lots differ from broker EOD position.
    PositionMismatch,
    /// Broker fill not present in OMS `seen_execs`.
    BrokerFillMissingInOms,
    /// OMS fill not present in broker EOD fill set.
    OmsFillMissingAtBroker,
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
            Self::VenueExecMissingInOms => "venue_exec_missing_in_oms",
            Self::OmsExecMissingAtVenue => "oms_exec_missing_at_venue",
            Self::OmsExecMissingInLedger => "oms_exec_missing_in_ledger",
            Self::LedgerFillWithoutOmsExec => "ledger_fill_without_oms_exec",
            Self::CashMismatch => "cash_mismatch",
            Self::PositionMismatch => "position_mismatch",
            Self::BrokerFillMissingInOms => "broker_fill_missing_in_oms",
            Self::OmsFillMissingAtBroker => "oms_fill_missing_at_broker",
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
    /// Compares OMS fill/working state to venue snapshots, Trade drop-copy, and ledger fills.
    ///
    /// Venue Trade history is current-session only (cleared on reconnect). When the
    /// drop-copy journal is empty, OMS→venue exec checks are skipped.
    #[must_use]
    pub fn reconcile(&self) -> ReconciliationReport {
        let mut mismatches = Vec::new();

        for order in self.orders().orders() {
            let id = order.id();
            let venue = self.venue_order(id);
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

        for v in self.venue_orders() {
            if self.orders().get(v.order_id).is_err() {
                mismatches.push(ReconciliationMismatch {
                    kind: ReconciliationKind::VenueOrderUnknownToOms,
                    order_id: v.order_id,
                    detail: "venue_inflight".into(),
                });
            }
        }

        self.reconcile_drop_copy(&mut mismatches);
        self.reconcile_ledger_fills(&mut mismatches);

        ReconciliationReport {
            ok: mismatches.is_empty(),
            mismatches,
        }
    }

    fn reconcile_drop_copy(&self, mismatches: &mut Vec<ReconciliationMismatch>) {
        let trades = self.venue_trade_execs();
        let mut venue_exec_ids = HashSet::new();
        for trade in &trades {
            venue_exec_ids.insert(trade.exec_id.as_str().to_owned());
            match self.orders().get(trade.order_id) {
                Ok(order) if order.seen_execs().iter().any(|e| e == &trade.exec_id) => {}
                Ok(_) | Err(_) => {
                    mismatches.push(ReconciliationMismatch {
                        kind: ReconciliationKind::VenueExecMissingInOms,
                        order_id: trade.order_id,
                        detail: format!(
                            "exec_id={} session={} seq={}",
                            trade.exec_id.as_str(),
                            trade.session.n,
                            trade.seq
                        ),
                    });
                }
            }
        }

        if trades.is_empty() {
            return;
        }

        for order in self.orders().orders() {
            for exec in order.seen_execs() {
                if !venue_exec_ids.contains(exec.as_str()) {
                    mismatches.push(ReconciliationMismatch {
                        kind: ReconciliationKind::OmsExecMissingAtVenue,
                        order_id: order.id(),
                        detail: format!("exec_id={}", exec.as_str()),
                    });
                }
            }
        }
    }

    fn reconcile_ledger_fills(&self, mismatches: &mut Vec<ReconciliationMismatch>) {
        let mut oms_exec_ids = HashSet::new();
        let mut exec_to_order = std::collections::HashMap::new();
        for order in self.orders().orders() {
            for exec in order.seen_execs() {
                let key = exec.as_str().to_owned();
                oms_exec_ids.insert(key.clone());
                exec_to_order.insert(key, order.id());
            }
        }

        let ledger_fill_ids: HashSet<String> = self
            .book()
            .journal()
            .entries()
            .filter_map(|(_, entry)| {
                entry
                    .idempotency_key()
                    .as_str()
                    .strip_prefix("fill:")
                    .map(str::to_owned)
            })
            .collect();

        for order in self.orders().orders() {
            for exec in order.seen_execs() {
                if !ledger_fill_ids.contains(exec.as_str()) {
                    mismatches.push(ReconciliationMismatch {
                        kind: ReconciliationKind::OmsExecMissingInLedger,
                        order_id: order.id(),
                        detail: format!("fill:{}", exec.as_str()),
                    });
                }
            }
        }

        for fill_id in &ledger_fill_ids {
            if !oms_exec_ids.contains(fill_id) {
                let order_id = exec_to_order
                    .get(fill_id)
                    .copied()
                    .unwrap_or_else(|| OrderId::from_u64(0));
                mismatches.push(ReconciliationMismatch {
                    kind: ReconciliationKind::LedgerFillWithoutOmsExec,
                    order_id,
                    detail: format!("fill:{fill_id}"),
                });
            }
        }
    }

    /// Compares internal cash / positions / fills to an external broker EOD snapshot.
    #[must_use]
    pub fn reconcile_eod(&self, snapshot: &BrokerEodSnapshot) -> ReconciliationReport {
        let mut mismatches = Vec::new();
        let zero_order = OrderId::from_u64(0);

        for row in &snapshot.cash {
            let available = self.book().available(row.account_id, row.currency);
            let reserved = self.book().reserved(row.account_id, row.currency);
            let internal = available
                .minor_units()
                .saturating_add(reserved.minor_units());
            if internal != row.minor_units {
                mismatches.push(ReconciliationMismatch {
                    kind: ReconciliationKind::CashMismatch,
                    order_id: zero_order,
                    detail: format!(
                        "account={} ccy={} internal={} broker={}",
                        row.account_id.get(),
                        row.currency.code().as_str(),
                        internal,
                        row.minor_units
                    ),
                });
            }
        }

        for row in &snapshot.positions {
            let lots = self.book().position(row.account_id, row.instrument_id);
            if lots != row.lots {
                mismatches.push(ReconciliationMismatch {
                    kind: ReconciliationKind::PositionMismatch,
                    order_id: zero_order,
                    detail: format!(
                        "account={} instrument={} internal={} broker={}",
                        row.account_id.get(),
                        row.instrument_id.get(),
                        lots,
                        row.lots
                    ),
                });
            }
        }

        let mut oms_execs: HashSet<String> = HashSet::new();
        let mut exec_to_order = std::collections::HashMap::new();
        for order in self.orders().orders() {
            for exec in order.seen_execs() {
                let key = exec.as_str().to_owned();
                oms_execs.insert(key.clone());
                exec_to_order.insert(key, order.id());
            }
        }

        // Empty `snapshot.fills` means "skip fill recon" (e.g. Alpaca auto-EOD positions-only).
        if !snapshot.fills.is_empty() {
            let mut broker_execs: HashSet<String> = HashSet::new();
            for fill in &snapshot.fills {
                let key = fill.exec_id.as_str().to_owned();
                broker_execs.insert(key.clone());
                if !oms_execs.contains(&key) {
                    mismatches.push(ReconciliationMismatch {
                        kind: ReconciliationKind::BrokerFillMissingInOms,
                        order_id: fill.order_id,
                        detail: format!("exec_id={}", fill.exec_id.as_str()),
                    });
                }
            }

            for (exec, order_id) in &exec_to_order {
                if !broker_execs.contains(exec) {
                    mismatches.push(ReconciliationMismatch {
                        kind: ReconciliationKind::OmsFillMissingAtBroker,
                        order_id: *order_id,
                        detail: format!("exec_id={exec}"),
                    });
                }
            }
        }

        ReconciliationReport {
            ok: mismatches.is_empty(),
            mismatches,
        }
    }
}

/// Broker end-of-day cash row (available + reserved expected total).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokerEodCash {
    /// Account.
    pub account_id: AccountId,
    /// Currency.
    pub currency: Currency,
    /// Total cash minor units (available + reserved).
    pub minor_units: i128,
}

/// Broker end-of-day position row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokerEodPosition {
    /// Account.
    pub account_id: AccountId,
    /// Instrument.
    pub instrument_id: InstrumentId,
    /// Signed lots.
    pub lots: i64,
}

/// Broker end-of-day fill row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokerEodFill {
    /// Internal order id when known.
    pub order_id: OrderId,
    /// Venue execution id.
    pub exec_id: ExecId,
    /// Fill quantity in lots.
    pub qty: i64,
    /// Fill price in ticks.
    pub price_ticks: i64,
}

/// External broker statement used for EOD reconciliation.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BrokerEodSnapshot {
    /// Cash balances by account/currency.
    pub cash: Vec<BrokerEodCash>,
    /// Position lots by account/instrument.
    pub positions: Vec<BrokerEodPosition>,
    /// Fills expected at the broker.
    pub fills: Vec<BrokerEodFill>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PaperEngine, SubmitRequest};
    use shinrai_exchange_simulator::FaultConfig;
    use shinrai_execution::{
        ExecType, ExecutionReport, ExecutionVenue, NewVenueOrder, SandboxConfig, SessionId,
    };
    use shinrai_instruments::{aapl, phase1_master, PriceTicks, QuantityLots};
    use shinrai_ledger::AccountId;
    use shinrai_money::{Currency, Money};
    use shinrai_orders::{ClientOrderId, ExecId, OrderId, Side, TimeInForce};
    use shinrai_risk::{RiskEngine, RiskLimits};

    fn funded_sandbox(auto_fill: bool) -> (PaperEngine, AccountId) {
        let config = if auto_fill {
            SandboxConfig::happy_path()
        } else {
            SandboxConfig::ack_only()
        };
        let mut engine =
            PaperEngine::with_sandbox(phase1_master(), config, RiskEngine::new(RiskLimits::demo()));
        let acc = AccountId::from_u64(1);
        engine
            .deposit(
                acc,
                Money::from_major(10_000, Currency::usd()).expect("d"),
                "dep",
            )
            .expect("dep");
        (engine, acc)
    }

    fn buy(acc: AccountId, cid: &str, qty: i64) -> SubmitRequest {
        SubmitRequest {
            account_id: acc,
            client_order_id: ClientOrderId::new(cid).expect("c"),
            instrument_id: aapl().id(),
            side: Side::Buy,
            qty: QuantityLots::from_lots(qty),
            price: PriceTicks::from_scaled(10_000),
            order_type: shinrai_orders::OrderType::Limit,
            time_in_force: TimeInForce::Gtc,
        }
    }

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
        engine.submit(&buy(acc, "r1", 5)).expect("s");
        assert!(engine.reconcile().ok);
    }

    #[test]
    fn venue_trade_not_drained_flags_missing_in_oms() {
        let (mut engine, acc) = funded_sandbox(false);
        let outcome = engine.submit(&buy(acc, "undrained", 2)).expect("s");
        let order = match outcome {
            shinrai_orders::SubmitOutcome::Created(o) => o,
            shinrai_orders::SubmitOutcome::Duplicate(_) => panic!("created"),
        };
        let venue_id = order.venue_order_id().cloned().expect("vid");
        let exec = ExecId::new("ORPHAN-E1").expect("e");
        engine
            .sandbox_mut()
            .expect("sbx")
            .inject(ExecutionReport::new(
                order.id(),
                venue_id,
                Some(exec),
                ExecType::Trade,
                QuantityLots::from_lots(1),
                PriceTicks::from_scaled(10_000),
                SessionId::new(1),
                99,
            ));
        let report = engine.reconcile();
        assert!(!report.ok);
        assert!(report.mismatches.iter().any(|m| {
            m.kind == ReconciliationKind::VenueExecMissingInOms && m.order_id == order.id()
        }));
    }

    #[test]
    fn oms_exec_missing_at_venue_after_session_reset_with_new_trades() {
        let (mut engine, acc) = funded_sandbox(true);
        engine.submit(&buy(acc, "first", 1)).expect("s");
        engine.reconnect_venue();
        engine.submit(&buy(acc, "second", 1)).expect("s");
        let report = engine.reconcile();
        assert!(!report.ok);
        assert!(report
            .mismatches
            .iter()
            .any(|m| m.kind == ReconciliationKind::OmsExecMissingAtVenue));
    }

    #[test]
    fn empty_drop_copy_after_reconnect_skips_oms_to_venue_exec_check() {
        let (mut engine, acc) = funded_sandbox(true);
        engine.submit(&buy(acc, "solo", 1)).expect("s");
        engine.reconnect_venue();
        // History empty → no OMS→venue exec mismatches; ledger still matches.
        let report = engine.reconcile();
        assert!(
            !report
                .mismatches
                .iter()
                .any(|m| m.kind == ReconciliationKind::OmsExecMissingAtVenue),
            "{report:?}"
        );
        assert!(report.ok, "{report:?}");
    }

    #[test]
    fn sandbox_trade_execs_visible_after_fill() {
        let mut sbx = shinrai_execution::SandboxBroker::new(SandboxConfig::happy_path());
        sbx.submit(&NewVenueOrder {
            order_id: OrderId::from_u64(3),
            instrument_id: aapl().id(),
            side: Side::Buy,
            qty: QuantityLots::from_lots(2),
            price: PriceTicks::from_scaled(10),
            tif: TimeInForce::Gtc,
        })
        .expect("submit");
        let _ = sbx.poll();
        let trades = ExecutionVenue::trade_execs(&sbx);
        assert_eq!(trades.len(), 1);
        assert_eq!(trades[0].order_id, OrderId::from_u64(3));
        assert!(!trades[0].exec_id.as_str().is_empty());
    }

    #[test]
    fn eod_cash_and_position_mismatch() {
        let mut engine = PaperEngine::new(phase1_master(), FaultConfig::happy_path());
        let acc = AccountId::from_u64(1);
        engine
            .deposit(
                acc,
                Money::from_major(10_000, Currency::usd()).expect("d"),
                "dep",
            )
            .expect("dep");
        engine.submit(&buy(acc, "eod1", 5)).expect("s");
        let total_cash = engine
            .book()
            .available(acc, Currency::usd())
            .minor_units()
            .saturating_add(engine.book().reserved(acc, Currency::usd()).minor_units());
        let fills: Vec<_> = engine
            .orders()
            .orders()
            .flat_map(|o| {
                o.seen_execs().iter().map(|e| BrokerEodFill {
                    order_id: o.id(),
                    exec_id: e.clone(),
                    qty: 5,
                    price_ticks: 10_000,
                })
            })
            .collect();
        let ok = engine.reconcile_eod(&BrokerEodSnapshot {
            cash: vec![BrokerEodCash {
                account_id: acc,
                currency: Currency::usd(),
                minor_units: total_cash,
            }],
            positions: vec![BrokerEodPosition {
                account_id: acc,
                instrument_id: aapl().id(),
                lots: 5,
            }],
            fills,
        });
        assert!(ok.ok, "{ok:?}");

        let bad = engine.reconcile_eod(&BrokerEodSnapshot {
            cash: vec![BrokerEodCash {
                account_id: acc,
                currency: Currency::usd(),
                minor_units: 1,
            }],
            positions: vec![BrokerEodPosition {
                account_id: acc,
                instrument_id: aapl().id(),
                lots: 99,
            }],
            fills: Vec::new(),
        });
        assert!(!bad.ok);
        assert!(bad
            .mismatches
            .iter()
            .any(|m| m.kind == ReconciliationKind::CashMismatch));
        assert!(bad
            .mismatches
            .iter()
            .any(|m| m.kind == ReconciliationKind::PositionMismatch));
    }
}
