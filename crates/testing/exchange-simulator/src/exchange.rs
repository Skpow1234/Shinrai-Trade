//! Simulated venue: inflight orders, scheduled reports, session lifecycle.

use std::collections::{BTreeMap, HashMap, VecDeque};

use shinrai_instruments::{InstrumentId, PriceTicks, QuantityLots};
use shinrai_orders::{ExecId, OrderId, Side, VenueOrderId};

use crate::clock::VirtualClock;
use crate::error::SimError;
use crate::faults::{FaultConfig, FillPolicy};
use crate::md::MdTick;
use crate::report::{ExecType, ExecutionReport, SessionId};

/// New order accepted by the simulator.
#[derive(Debug, Clone)]
pub struct NewSimOrder {
    /// Internal OMS order id.
    pub order_id: OrderId,
    /// Instrument.
    pub instrument_id: InstrumentId,
    /// Side.
    pub side: Side,
    /// Remaining / total quantity.
    pub qty: QuantityLots,
    /// Limit price.
    pub price: PriceTicks,
}

#[derive(Debug, Clone)]
struct SimOrder {
    order_id: OrderId,
    venue_order_id: VenueOrderId,
    #[allow(dead_code)]
    instrument_id: InstrumentId,
    qty: QuantityLots,
    price: PriceTicks,
    cum_qty: i64,
    canceled: bool,
}

/// Deterministic exchange simulator.
#[derive(Debug, Clone)]
pub struct SimExchange {
    clock: VirtualClock,
    connected: bool,
    session: SessionId,
    next_seq: u64,
    next_venue: u64,
    next_exec: u64,
    faults: FaultConfig,
    inflight: HashMap<OrderId, SimOrder>,
    /// Reports due at or after a clock time.
    scheduled: BTreeMap<u64, Vec<ExecutionReport>>,
    outbox: VecDeque<ExecutionReport>,
    md_seq: HashMap<InstrumentId, u64>,
}

/// Read-only view of a working order at the simulated venue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VenueOrderSnapshot {
    /// Internal OMS order id.
    pub order_id: OrderId,
    /// Original order quantity in lots.
    pub order_qty: i64,
    /// Cumulative filled quantity at the venue.
    pub cum_qty: i64,
    /// Cancel requested at the venue.
    pub canceled: bool,
}

impl Default for SimExchange {
    fn default() -> Self {
        Self::new(FaultConfig::happy_path())
    }
}

impl SimExchange {
    /// Creates a connected simulator with the given fault config.
    #[must_use]
    pub fn new(faults: FaultConfig) -> Self {
        Self {
            clock: VirtualClock::new(),
            connected: true,
            session: SessionId::new(1),
            next_seq: 1,
            next_venue: 1,
            next_exec: 1,
            faults,
            inflight: HashMap::new(),
            scheduled: BTreeMap::new(),
            outbox: VecDeque::new(),
            md_seq: HashMap::new(),
        }
    }

    /// Current logical time.
    #[must_use]
    pub const fn now(&self) -> u64 {
        self.clock.now()
    }

    /// Whether the session is up.
    #[must_use]
    pub const fn is_connected(&self) -> bool {
        self.connected
    }

    /// Current session id.
    #[must_use]
    pub const fn session(&self) -> SessionId {
        self.session
    }

    /// Updates fault configuration (does not replay history).
    pub fn set_faults(&mut self, faults: FaultConfig) {
        self.faults = faults;
    }

    /// Disconnects: new commands fail; queued reports stay until reconnect.
    pub fn disconnect(&mut self) {
        self.connected = false;
    }

    /// Reconnects on a new session id; sequence restarts; inflight orders remain.
    pub fn reconnect(&mut self) {
        self.connected = true;
        self.session = SessionId::new(self.session.n.saturating_add(1));
        self.next_seq = 1;
    }

    /// Submits an order. Immediate reports go to the outbox; delayed fills
    /// are scheduled on the clock.
    ///
    /// # Errors
    ///
    /// Returns [`SimError::Disconnected`] or identifier errors.
    pub fn submit(&mut self, order: &NewSimOrder) -> Result<(), SimError> {
        if !self.connected {
            return Err(SimError::Disconnected);
        }
        if order.qty.lots() <= 0 || order.price.scaled() <= 0 {
            return Err(SimError::InvalidQuantity);
        }

        let venue_order_id = VenueOrderId::new(format!("SIM-{}", self.next_venue))
            .map_err(|_| SimError::InvalidIdentifier)?;
        self.next_venue = self.next_venue.saturating_add(1);

        if self.faults.reject_all {
            let report = self.build_report(
                order.order_id,
                venue_order_id,
                None,
                ExecType::Rejected {
                    reason: "sim reject_all".into(),
                },
                order.qty,
                order.price,
            );
            self.outbox.push_back(report);
            return Ok(());
        }

        let sim = SimOrder {
            order_id: order.order_id,
            venue_order_id: venue_order_id.clone(),
            instrument_id: order.instrument_id,
            qty: order.qty,
            price: order.price,
            cum_qty: 0,
            canceled: false,
        };
        self.inflight.insert(order.order_id, sim);

        let ack = self.build_report(
            order.order_id,
            venue_order_id,
            None,
            ExecType::New,
            order.qty,
            order.price,
        );
        self.outbox.push_back(ack);
        self.schedule_fills(order.order_id)?;
        Ok(())
    }

    /// Requests cancel. Confirms immediately if working; optional late fill.
    ///
    /// # Errors
    ///
    /// Returns disconnected / unknown order errors.
    pub fn cancel(&mut self, order_id: OrderId) -> Result<(), SimError> {
        if !self.connected {
            return Err(SimError::Disconnected);
        }
        let Some(sim) = self.inflight.get(&order_id).cloned() else {
            return Err(SimError::UnknownOrder { id: order_id });
        };
        if sim.canceled {
            return Err(SimError::InvalidState("already canceled"));
        }
        let leaves = sim.qty.lots() - sim.cum_qty;
        if leaves <= 0 {
            let reject = self.build_report(
                order_id,
                sim.venue_order_id.clone(),
                None,
                ExecType::CancelReject {
                    reason: "already filled".into(),
                },
                QuantityLots::from_lots(0),
                sim.price,
            );
            self.outbox.push_back(reject);
            return Ok(());
        }

        let ack = self.build_report(
            order_id,
            sim.venue_order_id.clone(),
            None,
            ExecType::Canceled,
            QuantityLots::from_lots(leaves),
            sim.price,
        );
        self.outbox.push_back(ack);

        if let Some(sim_mut) = self.inflight.get_mut(&order_id) {
            sim_mut.canceled = true;
        }

        if self.faults.late_fill_after_cancel && leaves > 0 {
            let fill = self.trade_report(&sim, leaves, sim.price)?;
            let when = self.clock.now().saturating_add(1);
            self.scheduled.entry(when).or_default().push(fill);
        }
        Ok(())
    }

    /// Confirms a replace immediately (updates qty/price on the sim order).
    ///
    /// # Errors
    ///
    /// Returns disconnected / unknown / invalid qty errors.
    pub fn replace(
        &mut self,
        order_id: OrderId,
        new_qty: QuantityLots,
        new_price: PriceTicks,
    ) -> Result<(), SimError> {
        if !self.connected {
            return Err(SimError::Disconnected);
        }
        if new_qty.lots() <= 0 || new_price.scaled() <= 0 {
            return Err(SimError::InvalidQuantity);
        }
        let Some(sim) = self.inflight.get_mut(&order_id) else {
            return Err(SimError::UnknownOrder { id: order_id });
        };
        if sim.canceled {
            return Err(SimError::InvalidState("canceled"));
        }
        if new_qty.lots() < sim.cum_qty {
            return Err(SimError::InvalidQuantity);
        }
        sim.qty = new_qty;
        sim.price = new_price;
        let venue = sim.venue_order_id.clone();
        let report = self.build_report(
            order_id,
            venue,
            None,
            ExecType::Replaced,
            new_qty,
            new_price,
        );
        self.outbox.push_back(report);
        Ok(())
    }

    /// Advances the clock and moves due scheduled reports into the outbox.
    pub fn tick(&mut self, ticks: u64) {
        if ticks == 0 {
            return;
        }
        self.clock.advance(ticks);
        let now = self.clock.now();
        let due: Vec<u64> = self
            .scheduled
            .keys()
            .copied()
            .filter(|t| *t <= now)
            .collect();
        for t in due {
            if let Some(reports) = self.scheduled.remove(&t) {
                for r in reports {
                    self.outbox.push_back(r);
                }
            }
        }
    }

    /// Drains all currently queued execution reports (does not require connected).
    pub fn poll(&mut self) -> Vec<ExecutionReport> {
        self.outbox.drain(..).collect()
    }

    /// Venue view of one order (for reconciliation).
    #[must_use]
    pub fn venue_order(&self, order_id: OrderId) -> Option<VenueOrderSnapshot> {
        self.inflight.get(&order_id).map(|sim| VenueOrderSnapshot {
            order_id: sim.order_id,
            order_qty: sim.qty.lots(),
            cum_qty: sim.cum_qty,
            canceled: sim.canceled,
        })
    }

    /// All orders still tracked at the venue.
    pub fn venue_orders(&self) -> impl Iterator<Item = VenueOrderSnapshot> + '_ {
        self.inflight.values().map(|sim| VenueOrderSnapshot {
            order_id: sim.order_id,
            order_qty: sim.qty.lots(),
            cum_qty: sim.cum_qty,
            canceled: sim.canceled,
        })
    }

    /// Emits a market-data tick; may skip a seq if `md_skip_seq` is set.
    pub fn emit_md(&mut self, instrument_id: InstrumentId, price: PriceTicks) -> MdTick {
        let seq = self.md_seq.entry(instrument_id).or_insert(0);
        *seq = seq.saturating_add(1);
        if self.faults.md_skip_seq {
            *seq = seq.saturating_add(1);
        }
        MdTick {
            instrument_id,
            seq: *seq,
            price,
        }
    }

    fn schedule_fills(&mut self, order_id: OrderId) -> Result<(), SimError> {
        let sim = self
            .inflight
            .get(&order_id)
            .cloned()
            .ok_or(SimError::UnknownOrder { id: order_id })?;
        let leaves = sim.qty.lots() - sim.cum_qty;
        if leaves <= 0 {
            return Ok(());
        }

        match self.faults.fill_policy {
            FillPolicy::Rest => Ok(()),
            FillPolicy::Full => {
                let report = self.trade_report(&sim, leaves, sim.price)?;
                if self.faults.delay_ticks == 0 {
                    if let Some(s) = self.inflight.get_mut(&order_id) {
                        s.cum_qty = sim.qty.lots();
                    }
                }
                self.enqueue_fill(report);
                Ok(())
            }
            FillPolicy::Split { first_lots } => {
                if first_lots <= 0 || first_lots >= leaves {
                    return Err(SimError::InvalidQuantity);
                }
                let first = self.trade_report(&sim, first_lots, sim.price)?;
                if self.faults.delay_ticks == 0 {
                    if let Some(s) = self.inflight.get_mut(&order_id) {
                        s.cum_qty = first_lots;
                    }
                }
                self.enqueue_fill(first);
                let rest = leaves - first_lots;
                let rest_report = self.trade_report(&sim, rest, sim.price)?;
                let when = self
                    .clock
                    .now()
                    .saturating_add(self.faults.delay_ticks.max(1));
                self.scheduled.entry(when).or_default().push(rest_report);
                Ok(())
            }
        }
    }

    fn enqueue_fill(&mut self, report: ExecutionReport) {
        if self.faults.delay_ticks == 0 {
            if self.faults.duplicate_exec {
                self.outbox.push_back(report.clone());
            }
            self.outbox.push_back(report);
            return;
        }
        let when = self.clock.now().saturating_add(self.faults.delay_ticks);
        if self.faults.duplicate_exec {
            self.scheduled.entry(when).or_default().push(report.clone());
        }
        self.scheduled.entry(when).or_default().push(report);
    }

    fn trade_report(
        &mut self,
        sim: &SimOrder,
        qty: i64,
        price: PriceTicks,
    ) -> Result<ExecutionReport, SimError> {
        let exec_id = ExecId::new(format!("E-{}", self.next_exec))
            .map_err(|_| SimError::InvalidIdentifier)?;
        self.next_exec = self.next_exec.saturating_add(1);
        Ok(self.build_report(
            sim.order_id,
            sim.venue_order_id.clone(),
            Some(exec_id),
            ExecType::Trade,
            QuantityLots::from_lots(qty),
            price,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    fn build_report(
        &mut self,
        order_id: OrderId,
        venue_order_id: VenueOrderId,
        exec_id: Option<ExecId>,
        exec_type: ExecType,
        qty: QuantityLots,
        price: PriceTicks,
    ) -> ExecutionReport {
        let seq = self.next_seq;
        self.next_seq = self.next_seq.saturating_add(1);
        ExecutionReport::new(
            order_id,
            venue_order_id,
            exec_id,
            exec_type,
            qty,
            price,
            self.session,
            seq,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::stream_fingerprint;
    use shinrai_ledger::AccountId;
    use shinrai_orders::{
        ClientOrderId, CreateOrder, OrderError, OrderEvent, OrderStatus, OrderStore, SubmitOutcome,
    };

    fn new_order(id: u64, qty: i64) -> NewSimOrder {
        NewSimOrder {
            order_id: OrderId::from_u64(id),
            instrument_id: InstrumentId::from_u64(1),
            side: Side::Buy,
            qty: QuantityLots::from_lots(qty),
            price: PriceTicks::from_scaled(100),
        }
    }

    fn apply_reports(store: &mut OrderStore, reports: &[ExecutionReport]) {
        for r in reports {
            let Some(event) = r.to_order_event().expect("map") else {
                continue;
            };
            match store.apply_event(r.order_id(), event) {
                Ok(_)
                | Err(OrderError::IllegalTransition { .. } | OrderError::DuplicateExec { .. }) => {
                    // Late fills / duplicate execs do not mutate (defined policy).
                }
                Err(e) => panic!("unexpected oms error: {e}"),
            }
        }
    }

    #[test]
    fn submit_ack_partial_fill() {
        let mut sim = SimExchange::new(FaultConfig {
            fill_policy: FillPolicy::Split { first_lots: 4 },
            ..FaultConfig::happy_path()
        });
        let mut store = OrderStore::new();
        let created = match store
            .submit(&CreateOrder {
                account_id: AccountId::from_u64(1),
                client_order_id: ClientOrderId::new("c1").expect("c"),
                instrument_id: InstrumentId::from_u64(1),
                side: Side::Buy,
                order_qty: QuantityLots::from_lots(10),
                price: PriceTicks::from_scaled(100),
            })
            .expect("s")
        {
            SubmitOutcome::Created(o) => o,
            SubmitOutcome::Duplicate(_) => panic!("created"),
        };
        let oid = created.id();
        sim.submit(&NewSimOrder {
            order_id: oid,
            instrument_id: InstrumentId::from_u64(1),
            side: Side::Buy,
            qty: QuantityLots::from_lots(10),
            price: PriceTicks::from_scaled(100),
        })
        .expect("sim");

        let first = sim.poll();
        apply_reports(&mut store, &first);
        assert_eq!(
            store.get(oid).expect("g").status(),
            OrderStatus::PartiallyFilled
        );

        sim.tick(1);
        let rest = sim.poll();
        apply_reports(&mut store, &rest);
        assert_eq!(store.get(oid).expect("g").status(), OrderStatus::Filled);
        store.get(oid).expect("g").assert_invariants().expect("inv");
    }

    #[test]
    fn late_fill_after_cancel_is_rejected_by_oms() {
        let mut sim = SimExchange::new(FaultConfig {
            fill_policy: FillPolicy::Rest,
            late_fill_after_cancel: true,
            ..FaultConfig::happy_path()
        });
        let mut store = OrderStore::new();
        let created = match store
            .submit(&CreateOrder {
                account_id: AccountId::from_u64(1),
                client_order_id: ClientOrderId::new("c2").expect("c"),
                instrument_id: InstrumentId::from_u64(1),
                side: Side::Buy,
                order_qty: QuantityLots::from_lots(10),
                price: PriceTicks::from_scaled(100),
            })
            .expect("s")
        {
            SubmitOutcome::Created(o) => o,
            SubmitOutcome::Duplicate(_) => panic!("created"),
        };
        let oid = created.id();
        sim.submit(&new_order(oid.get(), 10)).expect("sim");
        apply_reports(&mut store, &sim.poll());
        store
            .apply_event(oid, OrderEvent::CancelRequested)
            .expect("cxl req");
        sim.cancel(oid).expect("sim cxl");
        apply_reports(&mut store, &sim.poll());
        assert_eq!(store.get(oid).expect("g").status(), OrderStatus::Canceled);

        sim.tick(1);
        let late = sim.poll();
        assert!(late
            .iter()
            .any(|r| matches!(r.exec_type(), ExecType::Trade)));
        apply_reports(&mut store, &late);
        assert_eq!(store.get(oid).expect("g").status(), OrderStatus::Canceled);
        assert_eq!(store.get(oid).expect("g").cum_qty().lots(), 0);
    }

    #[test]
    fn duplicate_exec_second_apply_is_noop() {
        let mut sim = SimExchange::new(FaultConfig {
            duplicate_exec: true,
            ..FaultConfig::happy_path()
        });
        let mut store = OrderStore::new();
        let created = match store
            .submit(&CreateOrder {
                account_id: AccountId::from_u64(1),
                client_order_id: ClientOrderId::new("c3").expect("c"),
                instrument_id: InstrumentId::from_u64(1),
                side: Side::Buy,
                order_qty: QuantityLots::from_lots(3),
                price: PriceTicks::from_scaled(50),
            })
            .expect("s")
        {
            SubmitOutcome::Created(o) => o,
            SubmitOutcome::Duplicate(_) => panic!("created"),
        };
        let oid = created.id();
        sim.submit(&NewSimOrder {
            order_id: oid,
            instrument_id: InstrumentId::from_u64(1),
            side: Side::Buy,
            qty: QuantityLots::from_lots(3),
            price: PriceTicks::from_scaled(50),
        })
        .expect("sim");
        apply_reports(&mut store, &sim.poll());
        let order = store.get(oid).expect("g");
        assert_eq!(order.status(), OrderStatus::Filled);
        assert_eq!(order.cum_qty().lots(), 3);
    }

    #[test]
    fn scripted_scenarios_are_deterministic() {
        let run = || {
            let mut sim = SimExchange::new(FaultConfig::happy_path());
            sim.submit(&new_order(1, 2)).expect("a");
            sim.submit(&new_order(2, 5)).expect("b");
            sim.poll()
        };
        let a = stream_fingerprint(&run());
        let b = stream_fingerprint(&run());
        assert_eq!(a, b);
        assert!(!a.is_empty());
    }

    #[test]
    fn disconnect_rejects_new_orders() {
        let mut sim = SimExchange::new(FaultConfig::happy_path());
        sim.disconnect();
        assert_eq!(sim.submit(&new_order(1, 1)), Err(SimError::Disconnected));
        sim.reconnect();
        sim.submit(&new_order(1, 1)).expect("after reconnect");
        assert_eq!(sim.session().n, 2);
    }

    #[test]
    fn md_gap_skips_sequence() {
        let mut sim = SimExchange::new(FaultConfig {
            md_skip_seq: true,
            ..FaultConfig::happy_path()
        });
        let t = sim.emit_md(InstrumentId::from_u64(1), PriceTicks::from_scaled(10));
        assert_eq!(t.seq, 2);
    }
}
