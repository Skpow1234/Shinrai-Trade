//! Paper engine orchestrating OMS, ledger, and simulated venue.

use std::collections::HashMap;

use shinrai_audit::{AuditKind, AuditLog};
use shinrai_exchange_simulator::FaultConfig;
use shinrai_execution::{NewVenueOrder, SandboxConfig};
use shinrai_instruments::InstrumentMaster;
use shinrai_ledger::{AccountId, BalancedEntry, LedgerError, PaperBook};
use shinrai_money::Money;
use shinrai_orders::{
    CreateOrder, DomainEffect, Order, OrderError, OrderEvent, OrderId, OrderStore, Side,
    SubmitOutcome,
};
use shinrai_risk::{RiskContext, RiskDecision, RiskEngine, RiskOrderIntent};

use crate::error::PaperError;
use crate::notional::notional;
use crate::venue::{VenueHandle, VenueKind};

/// Client request to submit a paper order.
#[derive(Debug, Clone)]
pub struct SubmitRequest {
    /// Account.
    pub account_id: AccountId,
    /// Client order id (idempotency with account).
    pub client_order_id: shinrai_orders::ClientOrderId,
    /// Instrument.
    pub instrument_id: shinrai_instruments::InstrumentId,
    /// Side (buy or sell).
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
    venue: VenueHandle,
    remaining_cash_reserve: HashMap<OrderId, Money>,
    remaining_position_reserve: HashMap<OrderId, i64>,
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

    /// Creates a session with an explicit risk engine (sim venue).
    #[must_use]
    pub fn with_risk(master: InstrumentMaster, faults: FaultConfig, risk: RiskEngine) -> Self {
        Self {
            master,
            book: PaperBook::new(),
            orders: OrderStore::new(),
            venue: VenueHandle::sim(faults),
            remaining_cash_reserve: HashMap::new(),
            remaining_position_reserve: HashMap::new(),
            risk,
            audit: AuditLog::new(),
            logical_now: 0,
        }
    }

    /// Creates a session backed by the in-process broker sandbox.
    #[must_use]
    pub fn with_sandbox(master: InstrumentMaster, config: SandboxConfig, risk: RiskEngine) -> Self {
        Self {
            master,
            book: PaperBook::new(),
            orders: OrderStore::new(),
            venue: VenueHandle::sandbox(config),
            remaining_cash_reserve: HashMap::new(),
            remaining_position_reserve: HashMap::new(),
            risk,
            audit: AuditLog::new(),
            logical_now: 0,
        }
    }

    /// Which venue backs this engine.
    #[must_use]
    pub const fn venue_kind(&self) -> VenueKind {
        self.venue.kind()
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

    /// Simulated venue (when [`VenueKind::Sim`]).
    #[must_use]
    pub fn sim(&self) -> Option<&shinrai_exchange_simulator::SimExchange> {
        self.venue.as_sim()
    }

    /// Mutable sandbox venue (when [`VenueKind::Sandbox`]) for injecting reports.
    pub fn sandbox_mut(&mut self) -> Option<&mut shinrai_execution::SandboxBroker> {
        self.venue.as_sandbox_mut()
    }

    /// Venue snapshot for one order (reconciliation).
    #[must_use]
    pub fn venue_order(&self, order_id: OrderId) -> Option<shinrai_execution::VenueOrderSnapshot> {
        self.venue.venue_order(order_id)
    }

    /// All venue-tracked orders (reconciliation).
    #[must_use]
    pub fn venue_orders(&self) -> Vec<shinrai_execution::VenueOrderSnapshot> {
        self.venue.venue_orders()
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

    /// Updates sim fault config when the venue is [`VenueKind::Sim`].
    pub fn set_sim_faults(&mut self, faults: FaultConfig) {
        if let VenueHandle::Sim(sim) = &mut self.venue {
            sim.set_faults(faults);
        }
    }

    /// Ensures new OMS ids do not collide with durable rows (call after attach/hydrate).
    pub fn bump_order_ids_past(&mut self, min_id: u64) {
        self.orders.bump_next_id_past(min_id);
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

    /// Replaces book/OMS/audit from durable snapshots (startup replay).
    ///
    /// Reinflates per-order cash/position leftovers and restores working orders
    /// into the in-process venue (no new execution reports). Pending OMS statuses
    /// (`PendingNew` / `PendingCancel` / `PendingReplace`) engage the global kill
    /// switch until an operator clears them.
    ///
    /// # Errors
    ///
    /// Returns ledger/order/venue errors while applying snapshots.
    pub fn hydrate(
        &mut self,
        ledger: impl IntoIterator<Item = BalancedEntry>,
        orders: impl IntoIterator<Item = Order>,
        audit: Vec<shinrai_audit::AuditRecord>,
        positions: impl IntoIterator<Item = (AccountId, shinrai_instruments::InstrumentId, i64, i64)>,
    ) -> Result<(), PaperError> {
        self.book = PaperBook::new();
        self.orders = OrderStore::new();
        self.remaining_cash_reserve.clear();
        self.remaining_position_reserve.clear();
        self.audit = AuditLog::new();

        for entry in ledger {
            let _ = self.book.restore_entry(entry)?;
        }
        for (account, instrument, lots, reserved) in positions {
            self.book.set_position(account, instrument, lots, reserved);
        }
        for order in orders {
            self.orders.restore_order(order);
        }
        self.audit.restore(audit);
        self.reinflate_working_state()?;
        Ok(())
    }

    /// Rebuilds leftover reserves and venue inflight from restored OMS rows.
    fn reinflate_working_state(&mut self) -> Result<(), PaperError> {
        use shinrai_orders::OrderStatus;

        let mut pending_ambiguous = false;
        let mut sell_reserved: HashMap<(AccountId, shinrai_instruments::InstrumentId), i64> =
            HashMap::new();

        let snapshots: Vec<Order> = self.orders.orders().cloned().collect();
        for order in &snapshots {
            if order.status().is_terminal() {
                continue;
            }

            if matches!(
                order.status(),
                OrderStatus::PendingNew | OrderStatus::PendingCancel | OrderStatus::PendingReplace
            ) {
                pending_ambiguous = true;
            }

            let leaves = order.leaves_qty();
            if leaves.lots() > 0 {
                match order.side() {
                    Side::Buy => {
                        let instrument = self.master.get(order.instrument_id())?;
                        let leftover = notional(instrument, order.price(), leaves)?;
                        self.remaining_cash_reserve.insert(order.id(), leftover);
                    }
                    Side::Sell => {
                        self.remaining_position_reserve
                            .insert(order.id(), leaves.lots());
                        *sell_reserved
                            .entry((order.account_id(), order.instrument_id()))
                            .or_insert(0) += leaves.lots();
                    }
                }
            }

            // Restore venue row for anything that had (or should have) venue state.
            if (order.status().is_working()
                || matches!(
                    order.status(),
                    OrderStatus::PendingCancel | OrderStatus::PendingReplace
                )
                || (order.status() == OrderStatus::PendingNew && order.venue_order_id().is_some()))
                && (leaves.lots() > 0 || order.cum_qty().lots() > 0)
            {
                self.venue.restore_working(order)?;
            }
        }

        // Align position reserved lots with open sell leaves when journal rows are stale.
        for ((account, instrument), reserved) in sell_reserved {
            let lots = self.book.position(account, instrument);
            self.book.set_position(account, instrument, lots, reserved);
        }

        if pending_ambiguous {
            self.risk.set_global_kill(true);
            tracing::warn!(
                "paper.hydrate: pending OMS statuses present; global kill switch engaged"
            );
        }

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
        tracing::debug!(
            account_id = req.account_id.get(),
            client_order_id = %req.client_order_id.as_str(),
            instrument_id = req.instrument_id.get(),
            side = ?req.side,
            qty = req.qty.lots(),
            price = req.price.scaled(),
            "paper.submit"
        );

        self.audit.record(
            self.logical_now,
            Some(req.account_id),
            None,
            AuditKind::OrderSubmitRequested,
        );

        let instrument = self.master.get(req.instrument_id)?;
        instrument.assert_tradable()?;
        instrument.assert_order_grid(req.price, req.qty)?;
        let order_notional = notional(instrument, req.price, req.qty)?;

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
            position_lots: self
                .book
                .available_position(req.account_id, req.instrument_id),
            notional: order_notional,
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
                match req.side {
                    Side::Buy => {
                        match self.book.reserve_for_order(
                            req.account_id,
                            order_notional,
                            format!("rsv:{}:{order_id}", req.account_id.get()),
                        ) {
                            Ok(_) => {
                                self.remaining_cash_reserve.insert(order_id, order_notional);
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
                    }
                    Side::Sell => {
                        match self.book.reserve_position_for_order(
                            req.account_id,
                            req.instrument_id,
                            req.qty.lots(),
                        ) {
                            Ok(()) => {
                                self.remaining_position_reserve
                                    .insert(order_id, req.qty.lots());
                                self.audit.record(
                                    self.logical_now,
                                    Some(req.account_id),
                                    Some(order_id),
                                    AuditKind::LedgerReserved,
                                );
                            }
                            Err(LedgerError::InsufficientPosition) => {
                                self.orders.apply_event(
                                    order_id,
                                    OrderEvent::Rejected {
                                        reason: "insufficient position".into(),
                                    },
                                )?;
                                return Err(PaperError::Ledger(LedgerError::InsufficientPosition));
                            }
                            Err(e) => return Err(PaperError::Ledger(e)),
                        }
                    }
                }

                self.venue.submit(&NewVenueOrder {
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
        tracing::debug!(order_id = order_id.get(), "paper.cancel");
        self.orders
            .apply_event(order_id, OrderEvent::CancelRequested)?;
        self.venue.cancel(order_id)?;
        self.drain()?;
        Ok(self.orders.get(order_id)?.clone())
    }

    /// Advances the venue clock and processes due reports (delayed fills).
    ///
    /// # Errors
    ///
    /// Returns OMS / ledger errors from draining.
    pub fn tick(&mut self, ticks: u64) -> Result<(), PaperError> {
        self.venue.tick(ticks);
        self.drain()
    }

    /// Polls the venue and applies reports to OMS + ledger.
    ///
    /// # Errors
    ///
    /// Returns mapping, OMS, or settle errors. Illegal transitions and duplicate
    /// execs are ignored (defined race / idempotency policy).
    pub fn drain(&mut self) -> Result<(), PaperError> {
        let reports = self.venue.poll();
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
                    let fee = Money::from_minor(0, fill_notional.currency());
                    match order.side() {
                        Side::Buy => {
                            let remaining = self
                                .remaining_cash_reserve
                                .get(&order_id)
                                .copied()
                                .ok_or(PaperError::ReservationShortfall { order_id })?;
                            if remaining.minor_units() < fill_notional.minor_units() {
                                return Err(PaperError::ReservationShortfall { order_id });
                            }
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
                                self.remaining_cash_reserve.remove(&order_id);
                            } else {
                                self.remaining_cash_reserve.insert(order_id, leftover);
                            }
                        }
                        Side::Sell => {
                            let remaining = self
                                .remaining_position_reserve
                                .get(&order_id)
                                .copied()
                                .ok_or(PaperError::ReservationShortfall { order_id })?;
                            if remaining < qty.lots() {
                                return Err(PaperError::ReservationShortfall { order_id });
                            }
                            self.book.settle_sell(
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
                            let leftover = remaining - qty.lots();
                            if leftover == 0 {
                                self.remaining_position_reserve.remove(&order_id);
                            } else {
                                self.remaining_position_reserve.insert(order_id, leftover);
                            }
                        }
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
        if let Some(amount) = self.remaining_cash_reserve.remove(&order_id) {
            if !amount.is_zero() {
                let account = self.orders.get(order_id)?.account_id();
                self.book.release_reserve(
                    account,
                    amount,
                    format!("rel:{}:{order_id}", account.get()),
                )?;
                self.audit.record(
                    self.logical_now,
                    Some(account),
                    Some(order_id),
                    AuditKind::LedgerReleased,
                );
            }
        }
        if let Some(qty) = self.remaining_position_reserve.remove(&order_id) {
            if qty > 0 {
                let order = self.orders.get(order_id)?;
                self.book.release_position_reserve(
                    order.account_id(),
                    order.instrument_id(),
                    qty,
                )?;
                self.audit.record(
                    self.logical_now,
                    Some(order.account_id()),
                    Some(order_id),
                    AuditKind::LedgerReleased,
                );
            }
        }
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

    fn aapl_order(
        acc: AccountId,
        clid: &str,
        side: Side,
        qty: i64,
        price_scaled: i64,
    ) -> SubmitRequest {
        SubmitRequest {
            account_id: acc,
            client_order_id: ClientOrderId::new(clid).expect("c"),
            instrument_id: aapl().id(),
            side,
            qty: QuantityLots::from_lots(qty),
            price: shinrai_instruments::PriceTicks::from_scaled(price_scaled),
        }
    }

    #[test]
    fn buy_fills_settles_once() {
        let (mut engine, acc) = funded_engine(FaultConfig::happy_path());
        let outcome = engine
            .submit(&aapl_order(acc, "c1", Side::Buy, 10, 10_000))
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
        assert!(engine.remaining_cash_reserve.is_empty());
        assert!(engine.remaining_position_reserve.is_empty());
    }

    #[test]
    fn duplicate_client_id_does_not_double_reserve() {
        let (mut engine, acc) = funded_engine(FaultConfig::happy_path());
        engine
            .submit(&aapl_order(acc, "dup", Side::Buy, 10, 10_000))
            .expect("s1");
        let cash = engine.book().available(acc, Currency::usd()).minor_units();
        let pos = engine.book().position(acc, aapl().id());
        let again = engine
            .submit(&aapl_order(acc, "dup", Side::Buy, 10, 10_000))
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
            .submit(&aapl_order(acc, "poor", Side::Buy, 10, 10_000))
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
            .submit(&aapl_order(acc, "cxl", Side::Buy, 10, 10_000))
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
    fn hydrate_reinflates_resting_buy_reserves_and_venue() {
        let (mut engine, acc) = funded_engine(FaultConfig {
            fill_policy: FillPolicy::Rest,
            ..FaultConfig::happy_path()
        });
        let outcome = engine
            .submit(&aapl_order(acc, "rest1", Side::Buy, 5, 10_000))
            .expect("submit");
        let order = match outcome {
            SubmitOutcome::Created(o) | SubmitOutcome::Duplicate(o) => o,
        };
        assert_eq!(order.status(), OrderStatus::New);
        assert!(!engine.remaining_cash_reserve.is_empty());

        let ledger: Vec<_> = engine
            .book()
            .journal()
            .entries()
            .map(|(_, e)| e.clone())
            .collect();
        let orders: Vec<_> = engine.orders().orders().cloned().collect();
        let audit = engine.audit().records().cloned().collect::<Vec<_>>();
        let positions: Vec<_> = engine.book().positions_iter().collect();

        let mut restarted = PaperEngine::new(
            phase1_master(),
            FaultConfig {
                fill_policy: FillPolicy::Rest,
                ..FaultConfig::happy_path()
            },
        );
        restarted
            .hydrate(ledger, orders, audit, positions)
            .expect("hydrate");

        assert!(restarted.remaining_cash_reserve.contains_key(&order.id()));
        assert!(restarted.venue_order(order.id()).is_some());
        assert!(restarted.reconcile().ok);

        let canceled = restarted.cancel(order.id()).expect("cancel after hydrate");
        assert_eq!(canceled.status(), OrderStatus::Canceled);
        assert!(restarted.book().reserved(acc, Currency::usd()).is_zero());
    }

    #[test]
    fn duplicate_exec_does_not_double_settle() {
        let (mut engine, acc) = funded_engine(FaultConfig {
            duplicate_exec: true,
            ..FaultConfig::happy_path()
        });
        engine
            .submit(&aapl_order(acc, "dex", Side::Buy, 10, 10_000))
            .expect("s");
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
    fn sell_after_buy_realizes_proceeds_and_reduces_position() {
        let (mut engine, acc) = funded_engine(FaultConfig::happy_path());
        engine
            .submit(&aapl_order(acc, "buy-s", Side::Buy, 10, 10_000))
            .expect("buy");
        let cash_after_buy = engine.book().available(acc, Currency::usd()).minor_units();
        let sell = engine
            .submit(&aapl_order(acc, "sell-s", Side::Sell, 4, 11_000))
            .expect("sell");
        let order = match sell {
            SubmitOutcome::Created(o) => o,
            SubmitOutcome::Duplicate(_) => panic!("created"),
        };
        assert_eq!(order.status(), OrderStatus::Filled);
        assert_eq!(engine.book().position(acc, aapl().id()), 6);
        assert!(engine.book().available(acc, Currency::usd()).minor_units() > cash_after_buy);
        assert!(engine.book().journal().trial_balance_ok());
    }

    #[test]
    fn sell_without_position_rejected_by_risk() {
        let (mut engine, acc) = funded_engine(FaultConfig::happy_path());
        let err = engine
            .submit(&aapl_order(acc, "naked", Side::Sell, 1, 10_000))
            .expect_err("no stock");
        assert!(matches!(err, PaperError::Risk(_)));
        assert!(engine.orders().is_empty());
    }

    #[test]
    fn sandbox_venue_fills_like_happy_path() {
        let mut engine = PaperEngine::with_sandbox(
            phase1_master(),
            shinrai_execution::SandboxConfig::happy_path(),
            RiskEngine::new(shinrai_risk::RiskLimits::demo()),
        );
        assert_eq!(engine.venue_kind(), VenueKind::Sandbox);
        let acc = AccountId::from_u64(1);
        engine
            .deposit(
                acc,
                Money::from_major(10_000, Currency::usd()).expect("d"),
                "dep",
            )
            .expect("dep");
        let outcome = engine
            .submit(&aapl_order(acc, "sbx-1", Side::Buy, 5, 10_000))
            .expect("submit");
        let order = match outcome {
            SubmitOutcome::Created(o) => o,
            SubmitOutcome::Duplicate(_) => panic!("created"),
        };
        assert_eq!(order.status(), OrderStatus::Filled);
        assert_eq!(engine.book().position(acc, aapl().id()), 5);
        assert!(engine.reconcile().ok);
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
