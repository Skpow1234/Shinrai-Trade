//! Ledger errors.

use core::fmt;

use shinrai_money::MoneyError;

/// Errors from posting, balancing, or paper-account commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LedgerError {
    /// Wrapped money arithmetic / currency error.
    Money(MoneyError),
    /// Entry had no postings.
    EmptyEntry,
    /// A posting amount was zero.
    ZeroAmount,
    /// Debit minor units did not equal credit minor units.
    Unbalanced {
        /// Total debit minor units.
        debit: i128,
        /// Total credit minor units.
        credit: i128,
    },
    /// Postings mixed more than one currency.
    MixedCurrency,
    /// Idempotency key was empty.
    EmptyIdempotencyKey,
    /// Available cash is not sufficient for the operation.
    InsufficientFunds,
    /// Reserved cash is not sufficient to settle or release.
    InsufficientReserved,
    /// Quantity for a position update was invalid (zero or overflow).
    InvalidQuantity,
    /// Available position is not sufficient to reserve for a sell.
    InsufficientPosition,
}

impl fmt::Display for LedgerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Money(err) => write!(f, "{err}"),
            Self::EmptyEntry => f.write_str("entry has no postings"),
            Self::ZeroAmount => f.write_str("posting amount must be non-zero"),
            Self::Unbalanced { debit, credit } => {
                write!(f, "unbalanced entry: debit {debit} != credit {credit}")
            }
            Self::MixedCurrency => f.write_str("entry mixes currencies"),
            Self::EmptyIdempotencyKey => f.write_str("idempotency key must be non-empty"),
            Self::InsufficientFunds => f.write_str("insufficient available funds"),
            Self::InsufficientReserved => f.write_str("insufficient reserved funds"),
            Self::InvalidQuantity => f.write_str("invalid position quantity"),
            Self::InsufficientPosition => f.write_str("insufficient available position"),
        }
    }
}

impl std::error::Error for LedgerError {}

impl From<MoneyError> for LedgerError {
    fn from(value: MoneyError) -> Self {
        Self::Money(value)
    }
}
