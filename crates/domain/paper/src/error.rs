//! Paper-loop errors.

use core::fmt;

use shinrai_exchange_simulator::SimError;
use shinrai_instruments::InstrumentError;
use shinrai_ledger::LedgerError;
use shinrai_money::MoneyError;
use shinrai_orders::{OrderError, OrderId};
use shinrai_risk::RiskRejectReason;

/// Errors from the paper trading loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaperError {
    /// Money arithmetic failed.
    Money(MoneyError),
    /// Instrument master / grid error.
    Instrument(InstrumentError),
    /// OMS error.
    Order(OrderError),
    /// Ledger error.
    Ledger(LedgerError),
    /// Simulated venue error.
    Sim(SimError),
    /// Only buy orders are wired to cash reserve / settle in Phase 1.
    UnsupportedSide,
    /// Fill notional was not exact in the quote currency scale.
    InexactNotional,
    /// Remaining reservation cannot cover this fill.
    ReservationShortfall {
        /// Order whose reservation was too small.
        order_id: OrderId,
    },
    /// Pre-trade risk rejected the order (OMS not mutated).
    Risk(RiskRejectReason),
}

impl fmt::Display for PaperError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Money(e) => write!(f, "{e}"),
            Self::Instrument(e) => write!(f, "{e}"),
            Self::Order(e) => write!(f, "{e}"),
            Self::Ledger(e) => write!(f, "{e}"),
            Self::Sim(e) => write!(f, "{e}"),
            Self::UnsupportedSide => f.write_str("only buy orders are supported in phase 1 paper"),
            Self::InexactNotional => f.write_str("notional is not exact in quote currency scale"),
            Self::ReservationShortfall { order_id } => {
                write!(f, "reservation shortfall for order {order_id}")
            }
            Self::Risk(r) => write!(f, "pre-trade risk rejected: {r}"),
        }
    }
}

impl std::error::Error for PaperError {}

impl From<MoneyError> for PaperError {
    fn from(value: MoneyError) -> Self {
        Self::Money(value)
    }
}

impl From<InstrumentError> for PaperError {
    fn from(value: InstrumentError) -> Self {
        Self::Instrument(value)
    }
}

impl From<OrderError> for PaperError {
    fn from(value: OrderError) -> Self {
        Self::Order(value)
    }
}

impl From<LedgerError> for PaperError {
    fn from(value: LedgerError) -> Self {
        Self::Ledger(value)
    }
}

impl From<SimError> for PaperError {
    fn from(value: SimError) -> Self {
        Self::Sim(value)
    }
}
