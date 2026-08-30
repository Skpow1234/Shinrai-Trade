//! Errors for money and currency operations.

use core::fmt;

/// Errors produced by currency parsing and monetary arithmetic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoneyError {
    /// Currency code was not exactly three ASCII letters.
    InvalidCurrencyCode,
    /// Requested scale exceeds the supported maximum.
    ScaleOutOfRange {
        /// The invalid scale that was requested.
        scale: u8,
    },
    /// Arithmetic overflowed `i128`.
    Overflow,
    /// Operands used different currencies.
    CurrencyMismatch {
        /// Left-hand currency code display (`AAA`).
        left: [u8; 3],
        /// Right-hand currency code display (`BBB`).
        right: [u8; 3],
    },
    /// Major-unit decimal string could not be parsed.
    InvalidAmount,
    /// Fractional part exceeded the currency scale.
    ScaleExceeded {
        /// Digits present in the fractional part.
        digits: usize,
        /// Currency scale.
        scale: u8,
    },
}

impl fmt::Display for MoneyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCurrencyCode => f.write_str("invalid currency code"),
            Self::ScaleOutOfRange { scale } => {
                write!(f, "currency scale {scale} exceeds maximum")
            }
            Self::Overflow => f.write_str("monetary arithmetic overflow"),
            Self::CurrencyMismatch { left, right } => write!(
                f,
                "currency mismatch: {} vs {}",
                core::str::from_utf8(left).unwrap_or("???"),
                core::str::from_utf8(right).unwrap_or("???"),
            ),
            Self::InvalidAmount => f.write_str("invalid monetary amount"),
            Self::ScaleExceeded { digits, scale } => {
                write!(
                    f,
                    "fractional digits {digits} exceed currency scale {scale}"
                )
            }
        }
    }
}

impl std::error::Error for MoneyError {}
