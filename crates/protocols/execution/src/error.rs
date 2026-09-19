//! Execution / venue errors.

use core::fmt;

use shinrai_orders::OrderId;

/// Errors from an execution venue (sim, sandbox, or live adapter).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionError {
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
    /// HTTP / wire transport failure (remote REST venue).
    Transport(String),
}

impl fmt::Display for ExecutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Disconnected => f.write_str("execution venue disconnected"),
            Self::UnknownOrder { id } => write!(f, "unknown venue order {id}"),
            Self::InvalidState(msg) => write!(f, "invalid venue order state: {msg}"),
            Self::InvalidQuantity => f.write_str("invalid quantity or price"),
            Self::InvalidIdentifier => f.write_str("invalid identifier"),
            Self::Transport(msg) => write!(f, "venue transport: {msg}"),
        }
    }
}

impl std::error::Error for ExecutionError {}
