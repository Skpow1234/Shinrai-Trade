//! Session-shaped licensed sandbox venue (`ExecutionVenue`).
//!
//! Models logon, heartbeats, outbound command sequencing, and inbound report
//! recovery over an in-process paper broker. Not a real FIX engine — trains the
//! OMS on session lifecycle before a vendor adapter is wired.

use std::collections::{HashMap, VecDeque};

use shinrai_instruments::{PriceTicks, QuantityLots};
use shinrai_orders::{ExecId, OrderId, TimeInForce, VenueOrderId};

use crate::error::ExecutionError;
use crate::report::{ExecType, ExecutionReport, SessionId};
use crate::session::VenueSessionState;
use crate::venue::{ExecutionVenue, NewVenueOrder, VenueOrderSnapshot, VenueTradeSnapshot};

/// Session phase for the licensed sandbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionPhase {
    /// No accepted logon.
    Disconnected,
    /// Logon accepted; commands and heartbeats allowed.
    Connected,
}

/// Behaviour + heartbeat knobs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LicensedSandboxConfig {
    /// When true, submit queues New then a full Trade immediately.
    pub auto_fill: bool,
    /// Logical ticks between outbound heartbeats.
    pub heartbeat_interval_ticks: u64,
    /// Disconnect when no inbound activity for this many ticks.
    pub heartbeat_timeout_ticks: u64,
}

impl Default for LicensedSandboxConfig {
    fn default() -> Self {
        Self {
            auto_fill: true,
            heartbeat_interval_ticks: 5,
            heartbeat_timeout_ticks: 15,
        }
    }
}

impl LicensedSandboxConfig {
    /// Happy-path: auto-fill + default heartbeats.
    #[must_use]
    pub const fn happy_path() -> Self {
        Self {
            auto_fill: true,
            heartbeat_interval_ticks: 5,
            heartbeat_timeout_ticks: 15,
        }
    }

    /// Ack only; fills injected via [`LicensedSandboxVenue::inject`].
    #[must_use]
    pub const fn ack_only() -> Self {
        Self {
            auto_fill: false,
            heartbeat_interval_ticks: 5,
            heartbeat_timeout_ticks: 15,
        }
    }
}

/// Outbound command kinds stored for resend tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutboundMsg {
    /// Session logon.
    Logon,
    /// Heartbeat.
    Heartbeat,
    /// New order.
    NewOrder {
        /// Internal order id.
        order_id: OrderId,
    },
    /// Cancel.
    Cancel {
        /// Internal order id.
        order_id: OrderId,
    },
    /// Replace.
    Replace {
        /// Internal order id.
        order_id: OrderId,
    },
    /// Logout.
    Logout,
}

#[derive(Debug, Clone)]
struct Inflight {
    venue_order_id: VenueOrderId,
    order_qty: i64,
    cum_qty: i64,
    price: PriceTicks,
    canceled: bool,
}

/// In-process licensed sandbox with explicit session lifecycle.
#[derive(Debug, Clone)]
pub struct LicensedSandboxVenue {
    config: LicensedSandboxConfig,
    phase: SessionPhase,
    session: SessionId,
    next_inbound_seq: u64,
    next_outbound_seq: u64,
    next_venue: u64,
    next_exec: u64,
    clock: u64,
    last_rx_tick: u64,
    last_tx_tick: u64,
    inflight: HashMap<OrderId, Inflight>,
    outbox: VecDeque<ExecutionReport>,
    history: Vec<ExecutionReport>,
    outbound_store: Vec<(u64, OutboundMsg)>,
}

impl Default for LicensedSandboxVenue {
    fn default() -> Self {
        Self::new(LicensedSandboxConfig::happy_path())
    }
}

impl LicensedSandboxVenue {
    /// Creates a **disconnected** venue (call [`Self::logon`] before trading).
    #[must_use]
    pub fn new(config: LicensedSandboxConfig) -> Self {
        Self {
            config,
            phase: SessionPhase::Disconnected,
            session: SessionId::new(1),
            next_inbound_seq: 1,
            next_outbound_seq: 1,
            next_venue: 1,
            next_exec: 1,
            clock: 0,
            last_rx_tick: 0,
            last_tx_tick: 0,
            inflight: HashMap::new(),
            outbox: VecDeque::new(),
            history: Vec::new(),
            outbound_store: Vec::new(),
        }
    }

    /// Connected happy-path venue (auto logon).
    #[must_use]
    pub fn happy_path() -> Self {
        let mut v = Self::new(LicensedSandboxConfig::happy_path());
        let _ = v.logon();
        v
    }

    /// Session phase.
    #[must_use]
    pub const fn phase(&self) -> SessionPhase {
        self.phase
    }

    /// Next outbound command sequence number.
    #[must_use]
    pub const fn next_outbound_seq(&self) -> u64 {
        self.next_outbound_seq
    }

    /// Stored outbound messages (for resend tests).
    #[must_use]
    pub fn outbound_store(&self) -> &[(u64, OutboundMsg)] {
        &self.outbound_store
    }

    /// Accepts logon; returns outbound seq assigned to the Logon message.
    ///
    /// # Errors
    ///
    /// Returns [`ExecutionError::InvalidState`] when already connected.
    pub fn logon(&mut self) -> Result<u64, ExecutionError> {
        if self.phase == SessionPhase::Connected {
            return Err(ExecutionError::InvalidState("already logged on"));
        }
        self.phase = SessionPhase::Connected;
        self.last_rx_tick = self.clock;
        self.last_tx_tick = self.clock;
        Ok(self.push_outbound(OutboundMsg::Logon))
    }

    /// Graceful logout.
    pub fn logout(&mut self) {
        if self.phase == SessionPhase::Connected {
            let _ = self.push_outbound(OutboundMsg::Logout);
        }
        self.phase = SessionPhase::Disconnected;
    }

    /// Replays outbound messages with seq in `[from_seq, to_seq]` (inclusive).
    #[must_use]
    pub fn resend(&self, from_seq: u64, to_seq: u64) -> Vec<(u64, OutboundMsg)> {
        self.outbound_store
            .iter()
            .filter(|(seq, _)| *seq >= from_seq && *seq <= to_seq)
            .cloned()
            .collect()
    }

    /// Queues an arbitrary report (tests / controlled fills).
    pub fn inject(&mut self, report: ExecutionReport) {
        if let ExecType::Trade = report.exec_type() {
            if let Some(row) = self.inflight.get_mut(&report.order_id()) {
                row.cum_qty = row
                    .cum_qty
                    .saturating_add(report.qty().lots())
                    .min(row.order_qty);
            }
        }
        if matches!(report.exec_type(), ExecType::Canceled) {
            if let Some(row) = self.inflight.get_mut(&report.order_id()) {
                row.canceled = true;
            }
        }
        self.note_rx();
        self.history.push(report.clone());
        self.outbox.push_back(report);
    }

    /// Restores a working order without emitting ack/fill reports (startup hydrate).
    ///
    /// # Errors
    ///
    /// Returns quantity / identifier errors.
    pub fn restore_working(
        &mut self,
        order_id: OrderId,
        order_qty: QuantityLots,
        price: PriceTicks,
        cum_qty: i64,
        venue_order_id: Option<VenueOrderId>,
    ) -> Result<(), ExecutionError> {
        if order_qty.lots() <= 0 || price.scaled() <= 0 {
            return Err(ExecutionError::InvalidQuantity);
        }
        if cum_qty < 0 || cum_qty > order_qty.lots() {
            return Err(ExecutionError::InvalidQuantity);
        }
        let venue_order_id = if let Some(id) = venue_order_id {
            id
        } else {
            let id = VenueOrderId::new(format!("LIC-{}", self.next_venue))
                .map_err(|_| ExecutionError::InvalidIdentifier)?;
            self.next_venue = self.next_venue.saturating_add(1);
            id
        };
        self.inflight.insert(
            order_id,
            Inflight {
                venue_order_id,
                order_qty: order_qty.lots(),
                cum_qty,
                price,
                canceled: false,
            },
        );
        Ok(())
    }

    fn push_outbound(&mut self, msg: OutboundMsg) -> u64 {
        let seq = self.next_outbound_seq;
        self.next_outbound_seq = self.next_outbound_seq.saturating_add(1);
        self.outbound_store.push((seq, msg));
        self.last_tx_tick = self.clock;
        seq
    }

    fn note_rx(&mut self) {
        self.last_rx_tick = self.clock;
    }

    fn require_connected(&self) -> Result<(), ExecutionError> {
        if self.phase != SessionPhase::Connected {
            return Err(ExecutionError::Disconnected);
        }
        Ok(())
    }

    fn push_report(
        &mut self,
        order_id: OrderId,
        venue_order_id: VenueOrderId,
        exec_id: Option<ExecId>,
        exec_type: ExecType,
        qty: QuantityLots,
        price: PriceTicks,
    ) {
        let seq = self.next_inbound_seq;
        self.next_inbound_seq = self.next_inbound_seq.saturating_add(1);
        let report = ExecutionReport::new(
            order_id,
            venue_order_id,
            exec_id,
            exec_type,
            qty,
            price,
            self.session,
            seq,
        );
        self.note_rx();
        self.history.push(report.clone());
        self.outbox.push_back(report);
    }

    fn alloc_venue_id(&mut self) -> Result<VenueOrderId, ExecutionError> {
        let id = VenueOrderId::new(format!("LIC-{}", self.next_venue))
            .map_err(|_| ExecutionError::InvalidIdentifier)?;
        self.next_venue = self.next_venue.saturating_add(1);
        Ok(id)
    }

    fn alloc_exec_id(&mut self) -> Result<ExecId, ExecutionError> {
        let id = ExecId::new(format!("LIC-E{}", self.next_exec))
            .map_err(|_| ExecutionError::InvalidIdentifier)?;
        self.next_exec = self.next_exec.saturating_add(1);
        Ok(id)
    }

    fn maybe_heartbeat(&mut self) {
        if self.phase != SessionPhase::Connected {
            return;
        }
        let interval = self.config.heartbeat_interval_ticks.max(1);
        if self.clock.saturating_sub(self.last_tx_tick) >= interval {
            let _ = self.push_outbound(OutboundMsg::Heartbeat);
        }
    }

    fn maybe_timeout(&mut self) {
        if self.phase != SessionPhase::Connected {
            return;
        }
        let timeout = self.config.heartbeat_timeout_ticks.max(1);
        if self.clock.saturating_sub(self.last_rx_tick) >= timeout {
            self.phase = SessionPhase::Disconnected;
        }
    }
}

impl ExecutionVenue for LicensedSandboxVenue {
    fn submit(&mut self, order: &NewVenueOrder) -> Result<(), ExecutionError> {
        self.require_connected()?;
        if order.qty.lots() <= 0 || order.price.scaled() <= 0 {
            return Err(ExecutionError::InvalidQuantity);
        }
        let _ = self.push_outbound(OutboundMsg::NewOrder {
            order_id: order.order_id,
        });
        let venue_order_id = self.alloc_venue_id()?;

        if order.tif == TimeInForce::Fok && !self.config.auto_fill {
            self.push_report(
                order.order_id,
                venue_order_id,
                None,
                ExecType::Rejected {
                    reason: "fok".into(),
                },
                order.qty,
                order.price,
            );
            return Ok(());
        }

        self.inflight.insert(
            order.order_id,
            Inflight {
                venue_order_id: venue_order_id.clone(),
                order_qty: order.qty.lots(),
                cum_qty: 0,
                price: order.price,
                canceled: false,
            },
        );
        self.push_report(
            order.order_id,
            venue_order_id.clone(),
            None,
            ExecType::New,
            order.qty,
            order.price,
        );
        if self.config.auto_fill {
            let exec_id = self.alloc_exec_id()?;
            if let Some(row) = self.inflight.get_mut(&order.order_id) {
                row.cum_qty = order.qty.lots();
            }
            self.push_report(
                order.order_id,
                venue_order_id,
                Some(exec_id),
                ExecType::Trade,
                order.qty,
                order.price,
            );
        } else if order.tif == TimeInForce::Ioc {
            if let Some(row) = self.inflight.get_mut(&order.order_id) {
                row.canceled = true;
            }
            self.push_report(
                order.order_id,
                venue_order_id,
                None,
                ExecType::Expired,
                order.qty,
                order.price,
            );
        }
        Ok(())
    }

    fn cancel(&mut self, order_id: OrderId) -> Result<(), ExecutionError> {
        self.require_connected()?;
        let _ = self.push_outbound(OutboundMsg::Cancel { order_id });
        let Some(row) = self.inflight.get_mut(&order_id) else {
            return Err(ExecutionError::UnknownOrder { id: order_id });
        };
        if row.canceled {
            return Err(ExecutionError::InvalidState("already canceled"));
        }
        if row.cum_qty >= row.order_qty {
            return Err(ExecutionError::InvalidState("already filled"));
        }
        row.canceled = true;
        let venue_order_id = row.venue_order_id.clone();
        let leaves = QuantityLots::from_lots(row.order_qty - row.cum_qty);
        let price = row.price;
        self.push_report(
            order_id,
            venue_order_id,
            None,
            ExecType::Canceled,
            leaves,
            price,
        );
        Ok(())
    }

    fn poll(&mut self) -> Vec<ExecutionReport> {
        self.outbox.drain(..).collect()
    }

    fn tick(&mut self, ticks: u64) {
        self.clock = self.clock.saturating_add(ticks);
        self.maybe_heartbeat();
        self.maybe_timeout();
    }

    fn venue_order(&self, order_id: OrderId) -> Option<VenueOrderSnapshot> {
        self.inflight.get(&order_id).map(|row| VenueOrderSnapshot {
            order_id,
            order_qty: row.order_qty,
            cum_qty: row.cum_qty,
            canceled: row.canceled,
        })
    }

    fn venue_orders(&self) -> Vec<VenueOrderSnapshot> {
        self.inflight
            .iter()
            .map(|(id, row)| VenueOrderSnapshot {
                order_id: *id,
                order_qty: row.order_qty,
                cum_qty: row.cum_qty,
                canceled: row.canceled,
            })
            .collect()
    }

    fn trade_execs(&self) -> Vec<VenueTradeSnapshot> {
        self.history
            .iter()
            .filter_map(|r| {
                if !matches!(r.exec_type(), ExecType::Trade) {
                    return None;
                }
                let exec_id = r.exec_id()?.clone();
                Some(VenueTradeSnapshot {
                    order_id: r.order_id(),
                    exec_id,
                    qty: r.qty().lots(),
                    price: r.price().scaled(),
                    session: r.session(),
                    seq: r.seq(),
                })
            })
            .collect()
    }

    fn replace(
        &mut self,
        order_id: OrderId,
        new_qty: QuantityLots,
        new_price: PriceTicks,
    ) -> Result<(), ExecutionError> {
        self.require_connected()?;
        if new_qty.lots() <= 0 || new_price.scaled() <= 0 {
            return Err(ExecutionError::InvalidQuantity);
        }
        let _ = self.push_outbound(OutboundMsg::Replace { order_id });
        let Some(row) = self.inflight.get_mut(&order_id) else {
            return Err(ExecutionError::UnknownOrder { id: order_id });
        };
        if row.canceled {
            return Err(ExecutionError::InvalidState("already canceled"));
        }
        if new_qty.lots() < row.cum_qty {
            return Err(ExecutionError::InvalidQuantity);
        }
        row.order_qty = new_qty.lots();
        row.price = new_price;
        let venue_order_id = row.venue_order_id.clone();
        self.push_report(
            order_id,
            venue_order_id,
            None,
            ExecType::Replaced,
            new_qty,
            new_price,
        );
        Ok(())
    }

    fn session_state(&self) -> VenueSessionState {
        if self.phase == SessionPhase::Connected {
            VenueSessionState::connected(self.session, self.next_inbound_seq)
        } else {
            VenueSessionState::disconnected(self.session, self.next_inbound_seq)
        }
    }

    fn disconnect(&mut self) {
        self.phase = SessionPhase::Disconnected;
    }

    fn reconnect(&mut self) {
        self.phase = SessionPhase::Connected;
        self.session = SessionId::new(self.session.n.saturating_add(1));
        self.next_inbound_seq = 1;
        self.next_outbound_seq = 1;
        self.history.clear();
        self.outbox.clear();
        self.outbound_store.clear();
        self.last_rx_tick = self.clock;
        self.last_tx_tick = self.clock;
        let _ = self.push_outbound(OutboundMsg::Logon);
    }

    fn poll_recovery(&mut self, from_seq: u64) -> Result<Vec<ExecutionReport>, ExecutionError> {
        self.require_connected()?;
        Ok(self
            .history
            .iter()
            .filter(|r| r.session() == self.session && r.seq() >= from_seq)
            .cloned()
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shinrai_instruments::InstrumentId;
    use shinrai_orders::Side;

    fn gtc_order(id: u64, qty: i64) -> NewVenueOrder {
        NewVenueOrder {
            order_id: OrderId::from_u64(id),
            instrument_id: InstrumentId::from_u64(1),
            side: Side::Buy,
            qty: QuantityLots::from_lots(qty),
            price: PriceTicks::from_scaled(10_000),
            tif: TimeInForce::Gtc,
        }
    }

    #[test]
    fn submit_before_logon_disconnected() {
        let mut v = LicensedSandboxVenue::new(LicensedSandboxConfig::happy_path());
        assert_eq!(v.phase(), SessionPhase::Disconnected);
        assert_eq!(
            v.submit(&gtc_order(1, 1)),
            Err(ExecutionError::Disconnected)
        );
    }

    #[test]
    fn logon_then_auto_fill() {
        let mut v = LicensedSandboxVenue::happy_path();
        assert_eq!(v.phase(), SessionPhase::Connected);
        v.submit(&gtc_order(1, 5)).expect("submit");
        let reports = v.poll();
        assert_eq!(reports.len(), 2);
        assert!(matches!(reports[0].exec_type(), ExecType::New));
        assert!(matches!(reports[1].exec_type(), ExecType::Trade));
        assert!(v
            .outbound_store()
            .iter()
            .any(|(_, m)| matches!(m, OutboundMsg::NewOrder { .. })));
    }

    #[test]
    fn heartbeat_and_timeout() {
        let mut v = LicensedSandboxVenue::new(LicensedSandboxConfig {
            auto_fill: false,
            heartbeat_interval_ticks: 3,
            heartbeat_timeout_ticks: 10,
        });
        v.logon().expect("logon");
        v.tick(3);
        assert!(v
            .outbound_store()
            .iter()
            .any(|(_, m)| matches!(m, OutboundMsg::Heartbeat)));
        // No inbound since logon; advance past timeout.
        v.tick(10);
        assert_eq!(v.phase(), SessionPhase::Disconnected);
        assert_eq!(
            v.submit(&gtc_order(2, 1)),
            Err(ExecutionError::Disconnected)
        );
    }

    #[test]
    fn resend_returns_outbound_window() {
        let mut v = LicensedSandboxVenue::happy_path();
        v.submit(&gtc_order(1, 1)).expect("submit");
        let _ = v.poll();
        let window = v.resend(1, 10);
        assert!(window.iter().any(|(_, m)| matches!(m, OutboundMsg::Logon)));
        assert!(window
            .iter()
            .any(|(_, m)| matches!(m, OutboundMsg::NewOrder { .. })));
    }

    #[test]
    fn reconnect_resets_seqs_and_allows_submit() {
        let mut v = LicensedSandboxVenue::happy_path();
        v.submit(&gtc_order(1, 1)).expect("s");
        let _ = v.poll();
        v.disconnect();
        assert!(!v.session_state().connected);
        v.reconnect();
        assert!(v.session_state().connected);
        assert_eq!(v.session_state().session.n, 2);
        assert_eq!(v.session_state().next_seq, 1);
        v.submit(&gtc_order(2, 1)).expect("after reconnect");
    }

    #[test]
    fn poll_recovery_from_seq() {
        let mut v = LicensedSandboxVenue::happy_path();
        v.submit(&gtc_order(9, 2)).expect("submit");
        let _ = v.poll();
        let gap = v.poll_recovery(2).expect("recovery");
        assert_eq!(gap.len(), 1);
        assert!(matches!(gap[0].exec_type(), ExecType::Trade));
    }
}
