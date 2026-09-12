//! Execution venue trait (sim, sandbox, or live broker adapter).

use shinrai_instruments::{InstrumentId, PriceTicks, QuantityLots};
use shinrai_orders::{OrderId, Side};

use crate::error::ExecutionError;
use crate::report::ExecutionReport;

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
}
