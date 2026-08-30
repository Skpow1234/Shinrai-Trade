//! Events that drive the order state machine and resulting domain effects.

use shinrai_instruments::{PriceTicks, QuantityLots};

use crate::ids::{ExecId, VenueOrderId};

/// Domain effects emitted by a successful transition (for later ledger wiring).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainEffect {
    /// Venue accepted the order.
    Accepted {
        /// Venue order id.
        venue_order_id: VenueOrderId,
    },
    /// Order rejected.
    Rejected {
        /// Human-readable reason.
        reason: String,
    },
    /// Trade applied.
    Trade {
        /// Execution id.
        exec_id: ExecId,
        /// Fill quantity.
        qty: QuantityLots,
        /// Fill price.
        price: PriceTicks,
        /// True when the order is now fully filled.
        filled: bool,
    },
    /// Cancel request recorded.
    CancelPending,
    /// Cancel confirmed.
    Canceled,
    /// Replace request recorded.
    ReplacePending,
    /// Replace confirmed.
    Replaced {
        /// New total order quantity.
        qty: QuantityLots,
        /// New limit price.
        price: PriceTicks,
    },
    /// Order expired.
    Expired,
}

/// Input event for [`crate::apply`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrderEvent {
    /// Venue acknowledges the order.
    Accepted {
        /// Venue-assigned id.
        venue_order_id: VenueOrderId,
    },
    /// Venue or risk rejects the order while pending new.
    Rejected {
        /// Reason text.
        reason: String,
    },
    /// Execution / fill report.
    Trade {
        /// Unique execution id.
        exec_id: ExecId,
        /// Fill quantity in lots.
        qty: QuantityLots,
        /// Fill price in ticks.
        price: PriceTicks,
    },
    /// Client requested cancel.
    CancelRequested,
    /// Venue confirmed cancel.
    Canceled,
    /// Client requested replace.
    ReplaceRequested {
        /// New total quantity (must be >= cum qty).
        new_qty: QuantityLots,
        /// New limit price.
        new_price: PriceTicks,
    },
    /// Venue confirmed replace.
    Replaced {
        /// Confirmed quantity.
        new_qty: QuantityLots,
        /// Confirmed price.
        new_price: PriceTicks,
    },
    /// Order expired (day / GTD).
    Expired,
}

impl OrderEvent {
    /// Stable name for error reporting and transition tests.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Accepted { .. } => "Accepted",
            Self::Rejected { .. } => "Rejected",
            Self::Trade { .. } => "Trade",
            Self::CancelRequested => "CancelRequested",
            Self::Canceled => "Canceled",
            Self::ReplaceRequested { .. } => "ReplaceRequested",
            Self::Replaced { .. } => "Replaced",
            Self::Expired => "Expired",
        }
    }
}
