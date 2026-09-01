//! Paper engine orchestrating OMS, ledger, and simulated venue.

use std::collections::HashMap;

use shinrai_audit::{AuditKind, AuditLog};
use shinrai_exchange_simulator::{FaultConfig, NewSimOrder, SimExchange};
use shinrai_instruments::InstrumentMaster;
use shinrai_ledger::{AccountId, LedgerError, PaperBook};
use shinrai_money::Money;
use shinrai_orders::{
    CreateOrder, DomainEffect, Order, OrderError, OrderEvent, OrderId, OrderStore, Side,
    SubmitOutcome,
};
use shinrai_risk::{RiskContext, RiskDecision, RiskEngine, RiskOrderIntent};

use crate::error::PaperError;
use crate::notional::notional;

/// Client request to submit a paper order.
#[derive(Debug, Clone)]
pub struct SubmitRequest {
    /// Account.
    pub account_id: AccountId,
    /// Client order id (idempotency with account).
    pub client_order_id: shinrai_orders::ClientOrderId,
    /// Instrument.
    pub instrument_id: shinrai_instruments::InstrumentId,
    /// Side ( buy only).
    pub side: Side,
    /// Quantity in lots.
    pub qty: shinrai_instruments::QuantityLots,
    /// Limit price in ticks.
    pub price: shinrai_instruments::PriceTicks,
}

/// Wired paper-trading session.
#[derive(Debug, Clone)]
pub struct PaperEngine {
    master: InstrumentMaster,
    book: PaperBook,
    orders: OrderStore,
    sim: SimExchange,
    remaining_reserve: HashMap<OrderId, Money>,
    risk: RiskEngine,
    audit: AuditLog,
    logical_now: u64,
}

impl PaperEngine {
    /// Creates a session with the given instrument master and venue faults.
    #[must_use]
    pub fn new(master: InstrumentMaster, faults: FaultConfig) -> Self {
        Self::with_risk(master, faults, RiskEngine::default())
    }

    /// Creates a session with an explicit risk engine.
    #[must_use]
    pub fn with_risk(master: InstrumentMaster, faults: FaultConfig, risk: RiskEngine) -> Self {
        Self {
            master,
            book: PaperBook::new(),
            orders: OrderStore::new(),
            sim: SimExchange::new(faults),
            remaining_reserve: HashMap::new(),
            risk,
            audit: AuditLog::new(),
            logical_now: 0,
        }
    }

    /// Instrument master.
    #[must_use]
    pub const fn master(&self) -> &InstrumentMaster {
        &self.master
    }

    /// Append-only audit trail.
    #[must_use]
    pub const fn audit(&self) -> &AuditLog {
        &self.audit
    }

    /// Sets logical time used for audit timestamps (unix seconds).
    pub const fn set_logical_now(&mut self, now: u64) {
        self.logical_now = now;
    }

    /// Paper book (cash / positions).
    #[must_use]
    pub const fn book(&self) -> &PaperBook {
        &self.book
    }

    /// Order store.
    #[must_use]
    pub const fn orders(&self) -> &OrderStore {
        &self.orders
    }

    /// Simulated venue.
    #[must_use]
    pub const fn sim(&self) -> &SimExchange {
        &self.sim
    }

    /// Pre-trade risk engine.
    #[must_use]
    pub const fn risk(&self) -> &RiskEngine {
        &self.risk
    }

    /// Mutable pre-trade risk engine.
    pub const fn risk_mut(&mut self) -> &mut RiskEngine {
        &mut self.risk
    }

    /// Paper deposit.
    ///
    /// # Errors
    ///
    /// Returns ledger errors.
    pub fn deposit(
        &mut self,
        account: AccountId,
        amount: Money,
        key: impl Into<String>,
    ) -> Result<(), PaperError> {
        self.book.deposit(account, amount, key)?;
        Ok(())
    }

    /// Submits an order: validate → risk → OMS → reserve → venue → drain reports.
    ///
    /// Duplicate `account + client_order_id` does not reserve twice.
    ///
    /// # Errors
    ///
    /// Returns validation, funds, OMS, or venue errors. Insufficient funds
    /// reject the OMS order and leave cash unchanged.
    #[allow(clippy::too_many_lines)]
    pub fn submit(&mut self, req: &SubmitRequest) -> Result<SubmitOutcome, PaperError> {
        self.audit.record(
            self.logical_now,
            Some(req.account_id),
            None,
            AuditKind::OrderSubmitRequested,
        );

        let instrument = self.master.get(req.instrument_id)?;
        instrument.assert_tradable()?;
        instrument.assert_order_grid(req.price, req.qty)?;
        let reserved_amt = notional(instrument, req.price, req.qty)?;

        if let Some(existing) = self
            .orders
            .get_by_client(req.account_id, &req.client_order_id)
        {
            self.audit.record(
                self.logical_now,
                Some(req.account_id),
                Some(existing.id()),
                AuditKind::OrderDuplicate,
            );
            return Ok(SubmitOutcome::Duplicate(existing.clone()));
        }

        let risk_ctx = RiskContext {
            available_cash: self
                .book
                .available(req.account_id, instrument.quote_currency()),
            position_lots: self.book.position(req.account_id, req.instrument_id),
            notional: reserved_amt,
        };
        let intent = RiskOrderIntent {
            account_id: req.account_id,
            instrument_id: req.instrument_id,
            side: req.side,
            qty: req.qty,
            price: req.price,
        };
        if let RiskDecision::Rejected(reason) = self.risk.check(&intent, &risk_ctx) {
            self.audit.record(
                self.logical_now,
                Some(req.account_id),
                None,
                AuditKind::RiskRejected {
                    code: reason.code().into(),
                },
            );
            return Err(PaperError::Risk(reason));
        }

        let create = CreateOrder {
            account_id: req.account_id,
            client_order_id: req.client_order_id.clone(),
            instrument_id: req.instrument_id,
            side: req.side,
            order_qty: req.qty,
            price: req.price,
        };
        let outcome = self.orders.submit(&create)?;
        match outcome {
            SubmitOutcome::Duplicate(order) => Ok(SubmitOutcome::Duplicate(order)),
            SubmitOutcome::Created(order) => {
                let order_id = order.id();
                self.audit.record(
                    self.logical_now,
                    Some(req.account_id),
                    Some(order_id),
                    AuditKind::OrderCreated,
                );
                match self.book.reserve_for_order(
                    req.account_id,
                    reserved_amt,
                    format!("rsv:{order_id}"),
                ) {
                    Ok(_) => {
                        self.remaining_reserve.insert(order_id, reserved_amt);
                        self.audit.record(
                            self.logical_now,
                            Some(req.account_id),
                            Some(order_id),
                            AuditKind::LedgerReserved,
                        );
                    }
                    Err(LedgerError::InsufficientFunds) => {
                        self.orders.apply_event(
                            order_id,
                            OrderEvent::Rejected {
                                reason: "insufficient funds".into(),
                            },
                        )?;
                        return Err(PaperError::Ledger(LedgerError::InsufficientFunds));
                    }
                    Err(e) => return Err(PaperError::Ledger(e)),
                }

                self.sim.submit(&NewSimOrder {
                    order_id,
                    instrument_id: req.instrument_id,
                    side: req.side,
                    qty: req.qty,
                    price: req.price,
                })?;
                self.audit.record(
                    self.logical_now,
                    Some(req.account_id),
                    Some(order_id),
                    AuditKind::VenueSubmitted,
                );
                self.drain()?;
                let order = self.orders.get(order_id)?.clone();
                Ok(SubmitOutcome::Created(order))
            }
        }
    }

    /// Requests cancel, asks the venue, then drains reports (releases leftover reserve).
    ///
    /// # Errors
    ///
    /// Returns OMS or venue errors.
    pub fn cancel(&mut self, order_id: OrderId) -> Result<Order, PaperError> {
        self.orders
            .apply_event(order_id, OrderEvent::CancelRequested)?;
        self.sim.cancel(order_id)?;
        self.drain()?;
        Ok(self.orders.get(order_id)?.clone())
    }

    /// Advances the venue clock and processes due reports (delayed fills).
    ///
    /// # Errors
    ///
    /// Returns OMS / ledger errors from draining.
    pub fn tick(&mut self, ticks: u64) -> Result<(), PaperError> {
        self.sim.tick(ticks);
        self.drain()
    }

    /// Polls the venue and applies reports to OMS + ledger.
    ///
    /// # Errors
    ///
    /// Returns mapping, OMS, or settle errors. Illegal transitions and duplicate
    /// execs are ignored (defined race / idempotency policy).
    pub fn drain(&mut self) -> Result<(), PaperError> {
        let reports = self.sim.poll();
        for report in reports {
            let exec_label = report.exec_type().name().to_owned();
            self.audit.record(
                self.logical_now,
                self.orders
                    .get(report.order_id())
                    .ok()
                    .map(Order::account_id),
                Some(report.order_id()),
                AuditKind::VenueReport {
                    exec_type: exec_label,
                },
            );
            let Some(event) = report.to_order_event()? else {
                continue;
            };
            let applied = match self.orders.apply_event(report.order_id(), event) {
                Ok(v) => v,
                Err(OrderError::IllegalTransition { .. } | OrderError::DuplicateExec { .. }) => {
                    continue;
                }
                Err(e) => return Err(PaperError::Order(e)),
            };
            self.audit.record(
                self.logical_now,
                self.orders
                    .get(report.order_id())
                    .ok()
                    .map(Order::account_id),
                Some(report.order_id()),
                AuditKind::OrderEventApplied {
                    status: applied.0.status().to_string(),
                },
            );
            self.apply_effects(report.order_id(), &applied.1)?;
        }
        Ok(())
    }

    fn apply_effects(
        &mut self,
        order_id: OrderId,
        effects: &[DomainEffect],
    ) -> Result<(), PaperError> {
        for effect in effects {
            match effect {
                DomainEffect::Trade {
                    exec_id,
                    qty,
                    price,
                    filled,
                } => {
                    let order = self.orders.get(order_id)?;
                    let instrument = self.master.get(order.instrument_id())?;
                    let fill_notional = notional(instrument, *price, *qty)?;
                    let remaining = self
                        .remaining_reserve
                        .get(&order_id)
                        .copied()
                        .ok_or(PaperError::ReservationShortfall { order_id })?;
                    if remaining.minor_units() < fill_notional.minor_units() {
                        return Err(PaperError::ReservationShortfall { order_id });
                    }
                    let fee = Money::from_minor(0, fill_notional.currency());
                    self.book.settle_buy(
                        order.account_id(),
                        order.instrument_id(),
                        qty.lots(),
                        fill_notional,
                        fee,
                        format!("fill:{exec_id}"),
                    )?;
                    self.audit.record(
                        self.logical_now,
                        Some(order.account_id()),
                        Some(order_id),
                        AuditKind::LedgerSettled,
                    );
                    let leftover = remaining.checked_sub(fill_notional)?;
                    if leftover.is_zero() {
                        self.remaining_reserve.remove(&order_id);
                    } else {
                        self.remaining_reserve.insert(order_id, leftover);
                    }
                    if *filled {
                        self.release_remaining(order_id)?;
                    }
                }
                DomainEffect::Rejected { .. } | DomainEffect::Canceled | DomainEffect::Expired => {
                    self.release_remaining(order_id)?;
                }
                DomainEffect::Accepted { .. }
                | DomainEffect::CancelPending
                | DomainEffect::ReplacePending
                | DomainEffect::Replaced { .. } => {}
            }
        }
        Ok(())
    }

    fn release_remaining(&mut self, order_id: OrderId) -> Result<(), PaperError> {
        let Some(amount) = self.remaining_reserve.remove(&order_id) else {
            return Ok(());
        };
        if amount.is_zero() {
            return Ok(());
        }
        let account = self.orders.get(order_id)?.account_id();
        self.book
            .release_reserve(account, amount, format!("rel:{order_id}"))?;
        self.audit.record(
            self.logical_now,
            Some(account),
            Some(order_id),
            AuditKind::LedgerReleased,
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shinrai_exchange_simulator::{FaultConfig, FillPolicy};
    use shinrai_instruments::{aapl, phase1_master, ExternalId, QuantityLots};
    use shinrai_money::{Currency, Money};
    use shinrai_orders::{ClientOrderId, OrderStatus};

    fn funded_engine(faults: FaultConfig) -> (PaperEngine, AccountId) {
        let mut engine = PaperEngine::new(phase1_master(), faults);
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

    fn aapl_buy(acc: AccountId, clid: &str, qty: i64, price_scaled: i64) -> SubmitRequest {
        SubmitRequest {
            account_id: acc,
            client_order_id: ClientOrderId::new(clid).expect("c"),
            instrument_id: aapl().id(),
            side: Side::Buy,
            qty: QuantityLots::from_lots(qty),
            price: shinrai_instruments::PriceTicks::from_scaled(price_scaled),
        }
    }

    #[test]
    fn buy_fills_settles_once() {
        let (mut engine, acc) = funded_engine(FaultConfig::happy_path());
        let outcome = engine
            .submit(&aapl_buy(acc, "c1", 10, 10_000))
            .expect("submit");
        let order = match outcome {
            SubmitOutcome::Created(o) => o,
            SubmitOutcome::Duplicate(_) => panic!("created"),
        };
        assert_eq!(order.status(), OrderStatus::Filled);
        assert_eq!(engine.book().position(acc, aapl().id()), 10);
        assert!(engine.book().reserved(acc, Currency::usd()).is_zero());
        // 10_000 - 1_000 notional
        assert_eq!(
            engine.book().available(acc, Currency::usd()).minor_units(),
            900_000
        );
        assert!(engine.book().journal().trial_balance_ok());
        assert!(engine.remaining_reserve.is_empty());
    }

    #[test]
    fn duplicate_client_id_does_not_double_reserve() {
        let (mut engine, acc) = funded_engine(FaultConfig::happy_path());
        engine
            .submit(&aapl_buy(acc, "dup", 10, 10_000))
            .expect("s1");
        let cash = engine.book().available(acc, Currency::usd()).minor_units();
        let pos = engine.book().position(acc, aapl().id());
        let again = engine
            .submit(&aapl_buy(acc, "dup", 10, 10_000))
            .expect("s2");
        assert!(matches!(again, SubmitOutcome::Duplicate(_)));
        assert_eq!(engine.orders().len(), 1);
        assert_eq!(
            engine.book().available(acc, Currency::usd()).minor_units(),
            cash
        );
        assert_eq!(engine.book().position(acc, aapl().id()), pos);
    }

    #[test]
    fn insufficient_funds_rejected_by_risk_before_oms() {
        let mut engine = PaperEngine::new(phase1_master(), FaultConfig::happy_path());
        let acc = AccountId::from_u64(1);
        engine
            .deposit(
                acc,
                Money::from_major(1, Currency::usd()).expect("d"),
                "dep",
            )
            .expect("dep");
        let err = engine
            .submit(&aapl_buy(acc, "poor", 10, 10_000))
            .expect_err("insuf");
        assert!(matches!(err, PaperError::Risk(_)));
        assert!(engine.orders().is_empty());
        assert_eq!(
            engine.book().available(acc, Currency::usd()).minor_units(),
            100
        );
        assert!(engine.book().reserved(acc, Currency::usd()).is_zero());
        assert_eq!(engine.book().position(acc, aapl().id()), 0);
    }

    #[test]
    fn cancel_releases_reserve() {
        let (mut engine, acc) = funded_engine(FaultConfig {
            fill_policy: FillPolicy::Rest,
            ..FaultConfig::happy_path()
        });
        let outcome = engine
            .submit(&aapl_buy(acc, "cxl", 10, 10_000))
            .expect("submit");
        let order = match outcome {
            SubmitOutcome::Created(o) => o,
            SubmitOutcome::Duplicate(_) => panic!("created"),
        };
        assert_eq!(order.status(), OrderStatus::New);
        assert_eq!(
            engine.book().reserved(acc, Currency::usd()).minor_units(),
            100_000
        );
        let canceled = engine.cancel(order.id()).expect("cxl");
        assert_eq!(canceled.status(), OrderStatus::Canceled);
        assert!(engine.book().reserved(acc, Currency::usd()).is_zero());
        assert_eq!(
            engine.book().available(acc, Currency::usd()).minor_units(),
            1_000_000
        );
        assert_eq!(engine.book().position(acc, aapl().id()), 0);
    }

    #[test]
    fn duplicate_exec_does_not_double_settle() {
        let (mut engine, acc) = funded_engine(FaultConfig {
            duplicate_exec: true,
            ..FaultConfig::happy_path()
        });
        engine.submit(&aapl_buy(acc, "dex", 10, 10_000)).expect("s");
        assert_eq!(engine.book().position(acc, aapl().id()), 10);
        assert_eq!(
            engine.book().available(acc, Currency::usd()).minor_units(),
            900_000
        );
    }

    #[test]
    fn invalid_qty_rejected_before_oms() {
        let (mut engine, acc) = funded_engine(FaultConfig::happy_path());
        let bad = SubmitRequest {
            account_id: acc,
            client_order_id: ClientOrderId::new("badq").expect("c"),
            instrument_id: aapl().id(),
            side: Side::Buy,
            qty: QuantityLots::from_lots(0),
            price: shinrai_instruments::PriceTicks::from_scaled(10_000),
        };
        assert!(engine.submit(&bad).is_err());
        assert!(engine.orders().is_empty());
    }

    #[test]
    fn alias_round_trip_still_in_master() {
        let engine = PaperEngine::new(phase1_master(), FaultConfig::happy_path());
        let ticker = ExternalId::ticker("AAPL").expect("t");
        assert_eq!(
            engine.master.resolve_alias(&ticker).expect("r"),
            aapl().id()
        );
    }
}
