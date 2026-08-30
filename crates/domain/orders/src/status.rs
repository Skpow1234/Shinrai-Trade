//! Order status enumeration (FIX-like `OrdStatus`).

use core::fmt;

/// Lifecycle status of an order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OrderStatus {
    /// Accepted by the OMS; not yet acknowledged by the venue.
    PendingNew,
    /// Working at the venue with zero fills.
    New,
    /// Working with at least one partial fill.
    PartiallyFilled,
    /// Fully filled (terminal).
    Filled,
    /// Cancel requested; waiting for venue ack.
    PendingCancel,
    /// Canceled (terminal).
    Canceled,
    /// Replace requested; waiting for venue ack.
    PendingReplace,
    /// Rejected before becoming working (terminal).
    Rejected,
    /// Expired (terminal).
    Expired,
}

impl OrderStatus {
    /// Returns true if the status is terminal (immutable).
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Filled | Self::Canceled | Self::Rejected | Self::Expired
        )
    }

    /// Returns true if the order may still receive trades.
    #[must_use]
    pub const fn is_working(self) -> bool {
        matches!(self, Self::New | Self::PartiallyFilled)
    }

    /// All statuses (for exhaustive transition tests).
    #[must_use]
    pub const fn all() -> [Self; 9] {
        [
            Self::PendingNew,
            Self::New,
            Self::PartiallyFilled,
            Self::Filled,
            Self::PendingCancel,
            Self::Canceled,
            Self::PendingReplace,
            Self::Rejected,
            Self::Expired,
        ]
    }
}

impl fmt::Display for OrderStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::PendingNew => "PendingNew",
            Self::New => "New",
            Self::PartiallyFilled => "PartiallyFilled",
            Self::Filled => "Filled",
            Self::PendingCancel => "PendingCancel",
            Self::Canceled => "Canceled",
            Self::PendingReplace => "PendingReplace",
            Self::Rejected => "Rejected",
            Self::Expired => "Expired",
        };
        f.write_str(s)
    }
}
