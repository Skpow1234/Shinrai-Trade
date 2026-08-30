//! Simulator errors.

use core::fmt;

use shinrai_orders::OrderId;

/// Errors from the simulated venue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SimError {
    /// Session is disconnected; new commands are not accepted.
    Disconnected,
    /// Unknown inflight order.
    UnknownOrder {
        /// Missing internal order id.
        id: OrderId,
    },
    /// Order is not in a state that accepts the command.
    InvalidState(&'static str),
    /// Quantity or price was not positive.
    InvalidQuantity,
    /// Identifier construction failed.
    InvalidIdentifier,
}

impl fmt::Display for SimError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Disconnected => f.write_str("simulator session disconnected"),
            Self::UnknownOrder { id } => write!(f, "unknown sim order {id}"),
            Self::InvalidState(msg) => write!(f, "invalid sim order state: {msg}"),
            Self::InvalidQuantity => f.write_str("invalid quantity or price"),
            Self::InvalidIdentifier => f.write_str("invalid identifier"),
        }
    }
}

impl std::error::Error for SimError {}
