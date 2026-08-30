//! Order-domain errors.

use core::fmt;

use crate::ids::{ExecId, OrderId};
use crate::status::OrderStatus;

/// Errors from order construction, transitions, or the order store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrderError {
    /// Empty client order id or other blank identifier.
    InvalidIdentifier,
    /// Quantity must be positive.
    InvalidQuantity,
    /// Price must be positive for limit orders.
    InvalidPrice,
    /// Transition is not allowed from the current status.
    IllegalTransition {
        /// Current status.
        from: OrderStatus,
        /// Event that was rejected.
        event: &'static str,
    },
    /// Trade quantity exceeds leaves quantity.
    Overfill {
        /// Requested fill lots.
        trade_lots: i64,
        /// Remaining lots.
        leaves_lots: i64,
    },
    /// Duplicate execution id on the same order.
    DuplicateExec {
        /// Duplicate id.
        exec_id: ExecId,
    },
    /// Replace quantity would be below already-filled quantity.
    ReplaceBelowFilled {
        /// Requested new order qty.
        new_qty: i64,
        /// Cumulative filled qty.
        cum_qty: i64,
    },
    /// Order id was not found in the store.
    UnknownOrder {
        /// Missing id.
        id: OrderId,
    },
}

impl fmt::Display for OrderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidIdentifier => f.write_str("invalid identifier"),
            Self::InvalidQuantity => f.write_str("quantity must be positive"),
            Self::InvalidPrice => f.write_str("price must be positive"),
            Self::IllegalTransition { from, event } => {
                write!(f, "illegal transition from {from} on {event}")
            }
            Self::Overfill {
                trade_lots,
                leaves_lots,
            } => write!(f, "overfill: trade {trade_lots} > leaves {leaves_lots}"),
            Self::DuplicateExec { exec_id } => write!(f, "duplicate exec id {exec_id}"),
            Self::ReplaceBelowFilled { new_qty, cum_qty } => {
                write!(f, "replace qty {new_qty} below filled {cum_qty}")
            }
            Self::UnknownOrder { id } => write!(f, "unknown order {id}"),
        }
    }
}

impl std::error::Error for OrderError {}
