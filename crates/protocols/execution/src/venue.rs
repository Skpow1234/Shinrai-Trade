//! Execution venue trait (sim, sandbox, or live broker adapter).

use shinrai_instruments::{InstrumentId, PriceTicks, QuantityLots};
use shinrai_orders::{ExecId, OrderId, Side};

use crate::error::ExecutionError;
use crate::report::{ExecutionReport, SessionId};
use crate::session::VenueSessionState;

/// New order accepted by a venue adapter.
#[derive(Debug, Clone)]
pub struct NewVenueOrder {
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

/// Read-only view of a working order at the venue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VenueOrderSnapshot {
    /// Internal OMS order id.
    pub order_id: OrderId,
    /// Original order quantity in lots.
    pub order_qty: i64,
    /// Cumulative filled quantity at the venue.
    pub cum_qty: i64,
    /// Cancel requested / confirmed at the venue.
    pub canceled: bool,
}

/// One Trade from the venue drop-copy / report journal (current session).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VenueTradeSnapshot {
    /// Internal OMS order id.
    pub order_id: OrderId,
    /// Venue execution id.
    pub exec_id: ExecId,
    /// Last fill quantity in lots.
    pub qty: i64,
    /// Fill price (scaled ticks).
    pub price: i64,
    /// Session that assigned the report.
    pub session: SessionId,
    /// Sequence within the session.
    pub seq: u64,
}

/// Venue that accepts orders and emits execution reports for the OMS.
pub trait ExecutionVenue {
    /// Submits an order to the venue.
    ///
    /// # Errors
    ///
    /// Returns disconnect / validation errors.
    fn submit(&mut self, order: &NewVenueOrder) -> Result<(), ExecutionError>;

    /// Requests cancel for an inflight order.
    ///
    /// # Errors
    ///
    /// Returns unknown-order or disconnect errors.
    fn cancel(&mut self, order_id: OrderId) -> Result<(), ExecutionError>;

    /// Drains ready execution reports (may be empty).
    fn poll(&mut self) -> Vec<ExecutionReport>;

    /// Advances a logical clock used for delayed reports (no-op if unused).
    fn tick(&mut self, ticks: u64);

    /// Snapshot of one venue order, if still known.
    fn venue_order(&self, order_id: OrderId) -> Option<VenueOrderSnapshot>;

    /// All known venue orders (for reconciliation).
    fn venue_orders(&self) -> Vec<VenueOrderSnapshot>;

    /// Current-session Trade reports (in-process drop-copy for recon).
    fn trade_execs(&self) -> Vec<VenueTradeSnapshot>;

    /// Current session cursor (connected flag, session id, next seq).
    fn session_state(&self) -> VenueSessionState;

    /// Marks the session disconnected (reject new commands; inflight retained).
    fn disconnect(&mut self);

    /// Reconnects on a new session id; sequence restarts; inflight retained.
    fn reconnect(&mut self);

    /// Returns reports for the current session with `seq >= from_seq` (gap fill).
    ///
    /// # Errors
    ///
    /// Returns [`ExecutionError::Disconnected`] when the session is down.
    fn poll_recovery(&mut self, from_seq: u64) -> Result<Vec<ExecutionReport>, ExecutionError>;
}
