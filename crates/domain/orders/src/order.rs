//! Order aggregate and fill accounting.

use shinrai_instruments::{InstrumentId, PriceTicks, QuantityLots};
use shinrai_ledger::AccountId;

use crate::error::OrderError;
use crate::ids::{ClientOrderId, ExecId, OrderId, VenueOrderId};
use crate::status::OrderStatus;

/// Buy or sell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    /// Buy.
    Buy,
    /// Sell.
    Sell,
}

/// Supported order types (Phase 1: limit).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OrderType {
    /// Limit order.
    Limit,
}

/// Order aggregate with FIX-like fill fields.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::struct_field_names)]
pub struct Order {
    id: OrderId,
    account_id: AccountId,
    client_order_id: ClientOrderId,
    instrument_id: InstrumentId,
    side: Side,
    order_type: OrderType,
    status: OrderStatus,
    order_qty: QuantityLots,
    price: PriceTicks,
    cum_qty: QuantityLots,
    leaves_qty: QuantityLots,
    /// Average fill price in scaled ticks, or `None` if unfilled.
    avg_px: Option<PriceTicks>,
    venue_order_id: Option<VenueOrderId>,
    reject_reason: Option<String>,
    seen_execs: Vec<ExecId>,
    /// Pending replace parameters while in [`OrderStatus::PendingReplace`].
    pending_replace_qty: Option<QuantityLots>,
    pending_replace_price: Option<PriceTicks>,
}

impl Order {
    /// Creates a new order in [`OrderStatus::PendingNew`].
    ///
    /// # Errors
    ///
    /// Returns an error if quantity or price is not positive.
    pub fn new_pending(
        id: OrderId,
        account_id: AccountId,
        client_order_id: ClientOrderId,
        instrument_id: InstrumentId,
        side: Side,
        order_qty: QuantityLots,
        price: PriceTicks,
    ) -> Result<Self, OrderError> {
        if order_qty.lots() <= 0 {
            return Err(OrderError::InvalidQuantity);
        }
        if price.scaled() <= 0 {
            return Err(OrderError::InvalidPrice);
        }
        Ok(Self {
            id,
            account_id,
            client_order_id,
            instrument_id,
            side,
            order_type: OrderType::Limit,
            status: OrderStatus::PendingNew,
            order_qty,
            price,
            cum_qty: QuantityLots::from_lots(0),
            leaves_qty: order_qty,
            avg_px: None,
            venue_order_id: None,
            reject_reason: None,
            seen_execs: Vec::new(),
            pending_replace_qty: None,
            pending_replace_price: None,
        })
    }

    /// Internal order id.
    #[must_use]
    pub const fn id(&self) -> OrderId {
        self.id
    }

    /// Account id.
    #[must_use]
    pub const fn account_id(&self) -> AccountId {
        self.account_id
    }

    /// Client order id.
    #[must_use]
    pub fn client_order_id(&self) -> &ClientOrderId {
        &self.client_order_id
    }

    /// Instrument id.
    #[must_use]
    pub const fn instrument_id(&self) -> InstrumentId {
        self.instrument_id
    }

    /// Side.
    #[must_use]
    pub const fn side(&self) -> Side {
        self.side
    }

    /// Order type.
    #[must_use]
    pub const fn order_type(&self) -> OrderType {
        self.order_type
    }

    /// Status.
    #[must_use]
    pub const fn status(&self) -> OrderStatus {
        self.status
    }

    /// Original / current order quantity.
    #[must_use]
    pub const fn order_qty(&self) -> QuantityLots {
        self.order_qty
    }

    /// Limit price.
    #[must_use]
    pub const fn price(&self) -> PriceTicks {
        self.price
    }

    /// Cumulative filled quantity.
    #[must_use]
    pub const fn cum_qty(&self) -> QuantityLots {
        self.cum_qty
    }

    /// Remaining quantity.
    #[must_use]
    pub const fn leaves_qty(&self) -> QuantityLots {
        self.leaves_qty
    }

    /// Average fill price, if any fills exist.
    #[must_use]
    pub const fn avg_px(&self) -> Option<PriceTicks> {
        self.avg_px
    }

    /// Venue order id, if acknowledged.
    #[must_use]
    pub fn venue_order_id(&self) -> Option<&VenueOrderId> {
        self.venue_order_id.as_ref()
    }

    /// Reject reason, if rejected.
    #[must_use]
    pub fn reject_reason(&self) -> Option<&str> {
        self.reject_reason.as_deref()
    }

    /// Asserts fill accounting invariants.
    ///
    /// # Errors
    ///
    /// Returns [`OrderError::InvalidQuantity`] if invariants are broken.
    pub fn assert_invariants(&self) -> Result<(), OrderError> {
        if self.cum_qty.lots() < 0 || self.leaves_qty.lots() < 0 {
            return Err(OrderError::InvalidQuantity);
        }
        if self.cum_qty.lots() > self.order_qty.lots() {
            return Err(OrderError::InvalidQuantity);
        }
        if self.cum_qty.lots() + self.leaves_qty.lots() != self.order_qty.lots() {
            return Err(OrderError::InvalidQuantity);
        }
        Ok(())
    }

    pub(crate) fn set_status(&mut self, status: OrderStatus) {
        self.status = status;
    }

    pub(crate) fn set_venue_order_id(&mut self, id: VenueOrderId) {
        self.venue_order_id = Some(id);
    }

    pub(crate) fn set_reject_reason(&mut self, reason: String) {
        self.reject_reason = Some(reason);
    }

    pub(crate) fn has_exec(&self, exec_id: &ExecId) -> bool {
        self.seen_execs.iter().any(|e| e == exec_id)
    }

    pub(crate) fn apply_trade(
        &mut self,
        exec_id: ExecId,
        qty: QuantityLots,
        price: PriceTicks,
    ) -> Result<bool, OrderError> {
        if self.has_exec(&exec_id) {
            return Err(OrderError::DuplicateExec { exec_id });
        }
        if qty.lots() <= 0 {
            return Err(OrderError::InvalidQuantity);
        }
        if price.scaled() <= 0 {
            return Err(OrderError::InvalidPrice);
        }
        if qty.lots() > self.leaves_qty.lots() {
            return Err(OrderError::Overfill {
                trade_lots: qty.lots(),
                leaves_lots: self.leaves_qty.lots(),
            });
        }

        let new_cum = self
            .cum_qty
            .lots()
            .checked_add(qty.lots())
            .ok_or(OrderError::InvalidQuantity)?;
        let new_leaves = self
            .order_qty
            .lots()
            .checked_sub(new_cum)
            .ok_or(OrderError::InvalidQuantity)?;

        // Weighted average in scaled ticks: (avg*cum + px*qty) / new_cum
        let prev_notional = self.avg_px.map_or(0_i128, |px| {
            i128::from(px.scaled()) * i128::from(self.cum_qty.lots())
        });
        let trade_notional = i128::from(price.scaled())
            .checked_mul(i128::from(qty.lots()))
            .ok_or(OrderError::InvalidQuantity)?;
        let total = prev_notional
            .checked_add(trade_notional)
            .ok_or(OrderError::InvalidQuantity)?;
        let avg = total
            .checked_div(i128::from(new_cum))
            .ok_or(OrderError::InvalidQuantity)?;
        let avg_i64 = i64::try_from(avg).map_err(|_| OrderError::InvalidQuantity)?;

        self.cum_qty = QuantityLots::from_lots(new_cum);
        self.leaves_qty = QuantityLots::from_lots(new_leaves);
        self.avg_px = Some(PriceTicks::from_scaled(avg_i64));
        self.seen_execs.push(exec_id);

        let filled = new_leaves == 0;
        if filled {
            self.status = OrderStatus::Filled;
        } else {
            self.status = OrderStatus::PartiallyFilled;
        }
        Ok(filled)
    }

    pub(crate) fn set_pending_replace(&mut self, qty: QuantityLots, price: PriceTicks) {
        self.pending_replace_qty = Some(qty);
        self.pending_replace_price = Some(price);
        self.status = OrderStatus::PendingReplace;
    }

    pub(crate) fn apply_replaced(
        &mut self,
        new_qty: QuantityLots,
        new_price: PriceTicks,
    ) -> Result<(), OrderError> {
        if new_qty.lots() <= 0 {
            return Err(OrderError::InvalidQuantity);
        }
        if new_price.scaled() <= 0 {
            return Err(OrderError::InvalidPrice);
        }
        if new_qty.lots() < self.cum_qty.lots() {
            return Err(OrderError::ReplaceBelowFilled {
                new_qty: new_qty.lots(),
                cum_qty: self.cum_qty.lots(),
            });
        }
        self.order_qty = new_qty;
        self.price = new_price;
        self.leaves_qty = QuantityLots::from_lots(new_qty.lots() - self.cum_qty.lots());
        self.pending_replace_qty = None;
        self.pending_replace_price = None;
        self.status = if self.cum_qty.lots() == 0 {
            OrderStatus::New
        } else if self.leaves_qty.lots() == 0 {
            OrderStatus::Filled
        } else {
            OrderStatus::PartiallyFilled
        };
        Ok(())
    }

    pub(crate) fn clear_pending_replace(&mut self) {
        self.pending_replace_qty = None;
        self.pending_replace_price = None;
    }
}
