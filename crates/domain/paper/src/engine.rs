//! Paper engine orchestrating OMS, ledger, and simulated venue.

use std::collections::HashMap;

use shinrai_audit::{AuditKind, AuditLog};
use shinrai_exchange_simulator::FaultConfig;
use shinrai_execution::{NewVenueOrder, SandboxConfig, SessionId, VenueSessionState};
use shinrai_instruments::{InstrumentId, InstrumentMaster, PriceTicks, QuantityLots};
use shinrai_ledger::{AccountId, BalancedEntry, LedgerError, PaperBook};
use shinrai_money::Money;
use shinrai_orders::{
    CreateOrder, DomainEffect, Order, OrderError, OrderEvent, OrderId, OrderStore, Side,
    SubmitOutcome,
};
use shinrai_risk::{RiskContext, RiskDecision, RiskEngine, RiskOrderIntent};

use crate::error::PaperError;
use crate::notional::notional;
use crate::risk_ctx::{
    realized_pnl_all_time, seed_marks_from_orders, unrealized_pnl_minor, utc_day_id,
};
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
    /// Limit / reference price in ticks.
    pub price: shinrai_instruments::PriceTicks,
    /// Order type (default Limit).
    pub order_type: shinrai_orders::OrderType,
    /// Time in force (default GTC).
    pub time_in_force: shinrai_orders::TimeInForce,
}

/// Client request to replace a working paper order.
#[derive(Debug, Clone)]
pub struct ReplaceRequest {
    /// Internal order id.
    pub order_id: OrderId,
    /// New total order quantity (must be ≥ cum qty).
    pub new_qty: shinrai_instruments::QuantityLots,
    /// New limit price.
    pub new_price: shinrai_instruments::PriceTicks,
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
    /// Last fill / mark prices for collar + MTM risk inputs.
    marks: HashMap<InstrumentId, PriceTicks>,
    /// UTC day id (`logical_now / 86400`) for day-P&L anchoring.
    risk_day_id: Option<u64>,
    /// All-time realized at the open of [`Self::risk_day_id`] per account.
    realized_at_day_open: HashMap<AccountId, i128>,
    /// Last applied venue session (None until first report).
    applied_session: Option<SessionId>,
    /// Next expected report sequence within [`Self::applied_session`].
    next_expected_seq: u64,
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
            marks: HashMap::new(),
            risk_day_id: None,
            realized_at_day_open: HashMap::new(),
            applied_session: None,
            next_expected_seq: 1,
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
            marks: HashMap::new(),
            risk_day_id: None,
            realized_at_day_open: HashMap::new(),
            applied_session: None,
            next_expected_seq: 1,
        }
    }

    /// Creates a session backed by the REST paper venue (local JSON broker).
    #[must_use]
    pub fn with_rest(master: InstrumentMaster, risk: RiskEngine) -> Self {
        Self {
            master,
            book: PaperBook::new(),
            orders: OrderStore::new(),
            venue: VenueHandle::rest_happy_path(),
            remaining_cash_reserve: HashMap::new(),
            remaining_position_reserve: HashMap::new(),
            risk,
            audit: AuditLog::new(),
            logical_now: 0,
            marks: HashMap::new(),
            risk_day_id: None,
            realized_at_day_open: HashMap::new(),
            applied_session: None,
            next_expected_seq: 1,
        }
    }

    /// Creates a session backed by a remote REST paper/broker (`SHINRAI_OG_REST_URL`).
    ///
    /// # Errors
    ///
    /// Returns venue transport errors when the HTTP client cannot be built.
    pub fn with_rest_remote(
        master: InstrumentMaster,
        risk: RiskEngine,
        base_url: impl Into<String>,
        bearer: Option<String>,
    ) -> Result<Self, PaperError> {
        Ok(Self {
            master,
            book: PaperBook::new(),
            orders: OrderStore::new(),
            venue: VenueHandle::rest_remote(base_url, bearer)?,
            remaining_cash_reserve: HashMap::new(),
            remaining_position_reserve: HashMap::new(),
            risk,
            audit: AuditLog::new(),
            logical_now: 0,
            marks: HashMap::new(),
            risk_day_id: None,
            realized_at_day_open: HashMap::new(),
            applied_session: None,
            next_expected_seq: 1,
        })
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

    /// Sets correlation id stamped on subsequent audit records.
    pub fn set_correlation_id(&mut self, id: Option<String>) {
        self.audit.set_correlation_id(id);
    }

    /// Verifies the in-memory audit hash chain.
    #[must_use]
    pub fn audit_chain_ok(&self) -> bool {
        self.audit.verify_chain()
    }

    /// Fill / mark prices used for risk MTM and exposure.
    #[must_use]
    pub const fn marks(&self) -> &HashMap<InstrumentId, PriceTicks> {
        &self.marks
    }

    /// Sets a mark used for collars / MTM / asset-class exposure.
    pub fn set_mark(&mut self, id: InstrumentId, price: PriceTicks) {
        if price.scaled() > 0 {
            self.marks.insert(id, price);
        }
    }

    /// Merges marks (e.g. gateway bootstrap / live MD) into the engine.
    pub fn merge_marks(&mut self, marks: impl IntoIterator<Item = (InstrumentId, PriceTicks)>) {
        for (id, px) in marks {
            self.set_mark(id, px);
        }
    }

    /// Builds pre-trade risk context (day P&L + asset-class exposure).
    fn build_risk_context(
        &mut self,
        account_id: AccountId,
        instrument_id: InstrumentId,
        order_notional: Money,
        limit_price: PriceTicks,
    ) -> Result<RiskContext, PaperError> {
        let instrument = self.master.get(instrument_id)?;
        let asset_class = instrument.asset_class();
        let quote = instrument.quote_currency();
        let day = utc_day_id(self.logical_now);
        if self.risk_day_id != Some(day) {
            self.risk_day_id = Some(day);
            self.realized_at_day_open.clear();
        }
        let all_time = realized_pnl_all_time(account_id, &self.orders, &self.master)?;
        let baseline = *self
            .realized_at_day_open
            .entry(account_id)
            .or_insert(all_time);
        let day_realized = all_time.saturating_sub(baseline);
        let unrealized = unrealized_pnl_minor(
            account_id,
            &self.book,
            &self.orders,
            &self.master,
            &self.marks,
        )?;
        let day_pnl_minor = day_realized.saturating_add(unrealized);
        let exposure = crate::risk_ctx::asset_class_exposure_minor(
            account_id,
            &self.book,
            &self.orders,
            &self.master,
            &self.marks,
            asset_class,
        )?;
        let ref_price = self
            .marks
            .get(&instrument_id)
            .copied()
            .or(Some(limit_price));
        Ok(RiskContext {
            available_cash: self.book.available(account_id, quote),
            position_lots: self.book.available_position(account_id, instrument_id),
            notional: order_notional,
            ref_price,
            now_unix: self.logical_now,
            day_pnl_minor,
            asset_class_exposure_minor: exposure,
        })
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

    /// Current-session venue Trade drop-copy (for reconciliation).
    #[must_use]
    pub fn venue_trade_execs(&self) -> Vec<shinrai_execution::VenueTradeSnapshot> {
        self.venue.trade_execs()
    }

    /// Venue session cursor (connected / session / next seq).
    #[must_use]
    pub fn venue_session(&self) -> VenueSessionState {
        self.venue.session_state()
    }

    /// Consumer-side applied session cursor (for ops / tests).
    #[must_use]
    pub const fn applied_venue_cursor(&self) -> (Option<SessionId>, u64) {
        (self.applied_session, self.next_expected_seq)
    }

    /// Disconnects the backing venue (tests / fault injection).
    pub fn disconnect_venue(&mut self) {
        self.venue.disconnect();
    }

    /// Reconnects the backing venue on a new session (tests / recovery).
    pub fn reconnect_venue(&mut self) {
        self.venue.reconnect();
        self.applied_session = None;
        self.next_expected_seq = 1;
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
        self.marks.clear();
        seed_marks_from_orders(&self.orders, &mut self.marks);
        self.risk_day_id = None;
        self.realized_at_day_open.clear();
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

        let risk_ctx =
            self.build_risk_context(req.account_id, req.instrument_id, order_notional, req.price)?;
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
            order_type: req.order_type,
            time_in_force: req.time_in_force,
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

                match self.venue.submit(&NewVenueOrder {
                    order_id,
                    instrument_id: req.instrument_id,
                    side: req.side,
                    qty: req.qty,
                    price: req.price,
                    tif: req.time_in_force,
                }) {
                    Ok(()) => {}
                    Err(e) => {
                        if matches!(e, shinrai_execution::ExecutionError::Disconnected) {
                            self.risk.set_global_kill(true);
                            tracing::warn!(
                                order_id = order_id.get(),
                                "paper.submit: venue disconnected after OMS create; kill switch engaged"
                            );
                        }
                        return Err(PaperError::Venue(e));
                    }
                }
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
        match self.venue.cancel(order_id) {
            Ok(()) => {}
            Err(e) => {
                if matches!(e, shinrai_execution::ExecutionError::Disconnected) {
                    self.risk.set_global_kill(true);
                    tracing::warn!(
                        order_id = order_id.get(),
                        "paper.cancel: venue disconnected; kill switch engaged"
                    );
                }
                return Err(PaperError::Venue(e));
            }
        }
        self.drain()?;
        Ok(self.orders.get(order_id)?.clone())
    }

    /// Requests replace of qty/price, asks the venue, then drains reports.
    ///
    /// Tops up cash/position reserves when the new leaves require more margin.
    ///
    /// # Errors
    ///
    /// Returns OMS, risk/funds, or venue errors.
    pub fn replace(&mut self, req: &ReplaceRequest) -> Result<Order, PaperError> {
        tracing::debug!(
            order_id = req.order_id.get(),
            new_qty = req.new_qty.lots(),
            new_price = req.new_price.scaled(),
            "paper.replace"
        );
        let order = self.orders.get(req.order_id)?.clone();
        let instrument = self.master.get(order.instrument_id())?;
        instrument.assert_tradable()?;
        instrument.assert_order_grid(req.new_price, req.new_qty)?;
        if req.new_qty.lots() < order.cum_qty().lots() {
            return Err(PaperError::Order(OrderError::ReplaceBelowFilled {
                new_qty: req.new_qty.lots(),
                cum_qty: order.cum_qty().lots(),
            }));
        }
        let new_leaves = QuantityLots::from_lots(req.new_qty.lots() - order.cum_qty().lots());
        match order.side() {
            Side::Buy => {
                let target = notional(instrument, req.new_price, new_leaves)?;
                let current = self
                    .remaining_cash_reserve
                    .get(&req.order_id)
                    .copied()
                    .unwrap_or_else(|| Money::from_minor(0, instrument.quote_currency()));
                if target.minor_units() > current.minor_units() {
                    let delta = target.checked_sub(current)?;
                    self.book.reserve_for_order(
                        order.account_id(),
                        delta,
                        format!(
                            "rsv-repl:{}:{}",
                            order.account_id().get(),
                            req.order_id.get()
                        ),
                    )?;
                    self.remaining_cash_reserve.insert(req.order_id, target);
                    self.audit.record(
                        self.logical_now,
                        Some(order.account_id()),
                        Some(req.order_id),
                        AuditKind::LedgerReserved,
                    );
                }
            }
            Side::Sell => {
                let current = self
                    .remaining_position_reserve
                    .get(&req.order_id)
                    .copied()
                    .unwrap_or(0);
                let need = new_leaves.lots();
                if need > current {
                    let delta = need - current;
                    self.book.reserve_position_for_order(
                        order.account_id(),
                        order.instrument_id(),
                        delta,
                    )?;
                    self.remaining_position_reserve.insert(req.order_id, need);
                    self.audit.record(
                        self.logical_now,
                        Some(order.account_id()),
                        Some(req.order_id),
                        AuditKind::LedgerReserved,
                    );
                }
            }
        }

        self.orders.apply_event(
            req.order_id,
            OrderEvent::ReplaceRequested {
                new_qty: req.new_qty,
                new_price: req.new_price,
            },
        )?;
        match self.venue.replace(req.order_id, req.new_qty, req.new_price) {
            Ok(()) => {}
            Err(e) => {
                if matches!(e, shinrai_execution::ExecutionError::Disconnected) {
                    self.risk.set_global_kill(true);
                    tracing::warn!(
                        order_id = req.order_id.get(),
                        "paper.replace: venue disconnected; kill switch engaged"
                    );
                }
                return Err(PaperError::Venue(e));
            }
        }
        self.drain()?;
        Ok(self.orders.get(req.order_id)?.clone())
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
    /// Detects sequence gaps within a session and calls [`VenueHandle::poll_recovery`].
    ///
    /// # Errors
    ///
    /// Returns mapping, OMS, or settle errors. Illegal transitions and duplicate
    /// execs are ignored (defined race / idempotency policy).
    pub fn drain(&mut self) -> Result<(), PaperError> {
        let reports = self.venue.poll();
        let reports = self.ensure_contiguous(reports)?;
        for report in reports {
            self.apply_report(&report)?;
        }
        Ok(())
    }

    fn ensure_contiguous(
        &mut self,
        mut reports: Vec<shinrai_execution::ExecutionReport>,
    ) -> Result<Vec<shinrai_execution::ExecutionReport>, PaperError> {
        if reports.is_empty() {
            return Ok(reports);
        }
        let first = &reports[0];
        if self.applied_session != Some(first.session()) {
            self.applied_session = Some(first.session());
            self.next_expected_seq = 1;
        }
        if first.seq() > self.next_expected_seq {
            let recovered = self.venue.poll_recovery(self.next_expected_seq)?;
            let mut merged = recovered;
            merged.append(&mut reports);
            reports = dedupe_session_seq(merged);
        } else if first.seq() < self.next_expected_seq {
            let expected = self.next_expected_seq;
            let session = self.applied_session;
            reports.retain(|r| Some(r.session()) == session && r.seq() >= expected);
        }
        Ok(reports)
    }

    fn apply_report(
        &mut self,
        report: &shinrai_execution::ExecutionReport,
    ) -> Result<(), PaperError> {
        if self.applied_session != Some(report.session()) {
            self.applied_session = Some(report.session());
            self.next_expected_seq = 1;
        }
        if report.seq() < self.next_expected_seq {
            return Ok(());
        }
        if report.seq() > self.next_expected_seq {
            // Should have been healed by ensure_contiguous; fail closed.
            self.risk.set_global_kill(true);
            tracing::warn!(
                expected = self.next_expected_seq,
                got = report.seq(),
                session = report.session().n,
                "paper.drain: unrecovered seq gap; kill switch engaged"
            );
            return Err(PaperError::Venue(
                shinrai_execution::ExecutionError::InvalidState("seq gap"),
            ));
        }

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
            self.next_expected_seq = report.seq().saturating_add(1);
            return Ok(());
        };
        let applied = match self.orders.apply_event(report.order_id(), event) {
            Ok(v) => v,
            Err(OrderError::IllegalTransition { .. } | OrderError::DuplicateExec { .. }) => {
                self.next_expected_seq = report.seq().saturating_add(1);
                return Ok(());
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
        self.next_expected_seq = report.seq().saturating_add(1);
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
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
                    let inst_id = self.orders.get(order_id)?.instrument_id();
                    if price.scaled() > 0 {
                        self.marks.insert(inst_id, *price);
                    }
                }
                DomainEffect::Rejected { .. } | DomainEffect::Canceled | DomainEffect::Expired => {
                    self.release_remaining(order_id)?;
                }
                DomainEffect::Replaced { qty: _, price } => {
                    let order = self.orders.get(order_id)?;
                    let instrument = self.master.get(order.instrument_id())?;
                    let leaves = order.leaves_qty();
                    match order.side() {
                        Side::Buy => {
                            let target = notional(instrument, *price, leaves)?;
                            let current = self
                                .remaining_cash_reserve
                                .remove(&order_id)
                                .unwrap_or_else(|| Money::from_minor(0, target.currency()));
                            if current.minor_units() > target.minor_units() {
                                let surplus = current.checked_sub(target)?;
                                self.book.release_reserve(
                                    order.account_id(),
                                    surplus,
                                    format!(
                                        "rel-repl:{}:{}",
                                        order.account_id().get(),
                                        order_id.get()
                                    ),
                                )?;
                                self.audit.record(
                                    self.logical_now,
                                    Some(order.account_id()),
                                    Some(order_id),
                                    AuditKind::LedgerReleased,
                                );
                            }
                            if target.is_zero() {
                                self.remaining_cash_reserve.remove(&order_id);
                            } else {
                                self.remaining_cash_reserve.insert(order_id, target);
                            }
                        }
                        Side::Sell => {
                            let target = leaves.lots();
                            let current = self
                                .remaining_position_reserve
                                .remove(&order_id)
                                .unwrap_or(0);
                            if current > target {
                                let surplus = current - target;
                                self.book.release_position_reserve(
                                    order.account_id(),
                                    order.instrument_id(),
                                    surplus,
                                )?;
                                self.audit.record(
                                    self.logical_now,
                                    Some(order.account_id()),
                                    Some(order_id),
                                    AuditKind::LedgerReleased,
                                );
                            }
                            if target == 0 {
                                self.remaining_position_reserve.remove(&order_id);
                            } else {
                                self.remaining_position_reserve.insert(order_id, target);
                            }
                        }
                    }
                }
                DomainEffect::Accepted { .. }
                | DomainEffect::CancelPending
                | DomainEffect::ReplacePending => {}
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

fn dedupe_session_seq(
    reports: Vec<shinrai_execution::ExecutionReport>,
) -> Vec<shinrai_execution::ExecutionReport> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(reports.len());
    for report in reports {
        let key = (report.session().n, report.seq());
        if seen.insert(key) {
            out.push(report);
        }
    }
    out
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
            order_type: shinrai_orders::OrderType::Limit,
            time_in_force: shinrai_orders::TimeInForce::Gtc,
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
    fn replace_resting_buy_updates_price_and_reserve() {
        use crate::ReplaceRequest;
        let (mut engine, acc) = funded_engine(FaultConfig {
            fill_policy: FillPolicy::Rest,
            ..FaultConfig::happy_path()
        });
        let outcome = engine
            .submit(&aapl_order(acc, "rest-repl", Side::Buy, 10, 10_000))
            .expect("submit");
        let order = match outcome {
            SubmitOutcome::Created(o) => o,
            SubmitOutcome::Duplicate(_) => panic!("created"),
        };
        assert_eq!(order.status(), OrderStatus::New);
        let replaced = engine
            .replace(&ReplaceRequest {
                order_id: order.id(),
                new_qty: QuantityLots::from_lots(6),
                new_price: shinrai_instruments::PriceTicks::from_scaled(9_000),
            })
            .expect("replace");
        assert_eq!(replaced.status(), OrderStatus::New);
        assert_eq!(replaced.order_qty().lots(), 6);
        assert_eq!(replaced.price().scaled(), 9_000);
        assert_eq!(replaced.leaves_qty().lots(), 6);
        // Remaining reserve should match leaves * new price (AAPL tick/lot → USD).
        let reserved = engine.book().reserved(acc, Currency::usd()).minor_units();
        // 6 lots * 9000 ticks → same notional helper as paper uses for AAPL.
        let expected = crate::notional(
            engine.master().get(aapl().id()).expect("i"),
            shinrai_instruments::PriceTicks::from_scaled(9_000),
            QuantityLots::from_lots(6),
        )
        .expect("n")
        .minor_units();
        assert_eq!(reserved, expected);
        assert!(engine.reconcile().ok);
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
            order_type: shinrai_orders::OrderType::Limit,
            time_in_force: shinrai_orders::TimeInForce::Gtc,
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
    fn rest_venue_fills_like_happy_path() {
        let mut engine = PaperEngine::with_rest(
            phase1_master(),
            RiskEngine::new(shinrai_risk::RiskLimits::demo()),
        );
        assert_eq!(engine.venue_kind(), VenueKind::Rest);
        let acc = AccountId::from_u64(1);
        engine
            .deposit(
                acc,
                Money::from_major(10_000, Currency::usd()).expect("d"),
                "dep",
            )
            .expect("dep");
        let outcome = engine
            .submit(&aapl_order(acc, "rest-1", Side::Buy, 5, 10_000))
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
    fn disconnect_after_create_engages_kill_switch() {
        let mut engine = PaperEngine::with_sandbox(
            phase1_master(),
            shinrai_execution::SandboxConfig::ack_only(),
            RiskEngine::new(shinrai_risk::RiskLimits::demo()),
        );
        let acc = AccountId::from_u64(1);
        engine
            .deposit(
                acc,
                Money::from_major(10_000, Currency::usd()).expect("d"),
                "dep",
            )
            .expect("dep");
        engine.disconnect_venue();
        let err = engine
            .submit(&aapl_order(acc, "ambig-1", Side::Buy, 1, 10_000))
            .expect_err("disconnected");
        assert!(matches!(
            err,
            PaperError::Venue(shinrai_execution::ExecutionError::Disconnected)
        ));
        assert!(matches!(
            engine.risk().check(
                &RiskOrderIntent {
                    account_id: acc,
                    instrument_id: aapl().id(),
                    side: Side::Buy,
                    qty: QuantityLots::from_lots(1),
                    price: shinrai_instruments::PriceTicks::from_scaled(10_000),
                },
                &RiskContext {
                    available_cash: engine.book().available(acc, Currency::usd()),
                    position_lots: 0,
                    notional: Money::from_major(1, Currency::usd()).expect("n"),
                    ref_price: None,
                    now_unix: 0,
                    day_pnl_minor: 0,
                    asset_class_exposure_minor: 0,
                }
            ),
            RiskDecision::Rejected(_)
        ));
        let pending = engine
            .orders()
            .orders()
            .filter(|o| o.status() == OrderStatus::PendingNew)
            .count();
        assert_eq!(pending, 1);
    }

    #[test]
    fn ioc_on_resting_sandbox_expires() {
        let mut engine = PaperEngine::with_sandbox(
            phase1_master(),
            shinrai_execution::SandboxConfig::ack_only(),
            RiskEngine::new(shinrai_risk::RiskLimits::demo()),
        );
        let acc = AccountId::from_u64(1);
        engine
            .deposit(
                acc,
                Money::from_major(10_000, Currency::usd()).expect("d"),
                "dep",
            )
            .expect("dep");
        let mut req = aapl_order(acc, "ioc-1", Side::Buy, 5, 10_000);
        req.time_in_force = shinrai_orders::TimeInForce::Ioc;
        let outcome = engine.submit(&req).expect("submit");
        let order = match outcome {
            SubmitOutcome::Created(o) => o,
            SubmitOutcome::Duplicate(_) => panic!("created"),
        };
        assert_eq!(order.status(), OrderStatus::Expired);
        assert_eq!(order.cum_qty().lots(), 0);
        assert_eq!(engine.book().position(acc, aapl().id()), 0);
        assert!(engine.book().reserved(acc, Currency::usd()).is_zero());
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
