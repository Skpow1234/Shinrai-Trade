//! Errors for instrument construction, grid checks, and master lookups.

use core::fmt;

use crate::ids::InstrumentId;

/// Errors from instrument reference data and trading-grid validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstrumentError {
    /// Empty or otherwise invalid symbol / identifier text.
    InvalidIdentifier,
    /// ISIN failed format or check-digit validation.
    InvalidIsin,
    /// MIC must be exactly four alphanumeric characters.
    InvalidMic,
    /// Scale exceeds the supported maximum.
    ScaleOutOfRange {
        /// Requested scale.
        scale: u8,
    },
    /// Tick or lot table configuration is inconsistent.
    InvalidGrid(&'static str),
    /// Decimal string could not be parsed.
    InvalidDecimal,
    /// Fractional digits exceed the instrument scale.
    ScaleExceeded {
        /// Digits present.
        digits: usize,
        /// Allowed scale.
        scale: u8,
    },
    /// Arithmetic overflowed.
    Overflow,
    /// Price is not aligned to the applicable tick size.
    PriceOffGrid {
        /// Scaled price that failed validation.
        scaled: i64,
        /// Required tick increment in scaled units.
        tick_size_scaled: i64,
    },
    /// Quantity is not aligned to the lot / step size.
    QuantityOffGrid {
        /// Scaled quantity that failed validation.
        scaled: i64,
        /// Required step in scaled units.
        step_scaled: i64,
    },
    /// Quantity is below the instrument minimum.
    QuantityBelowMin {
        /// Scaled quantity.
        scaled: i64,
        /// Minimum scaled quantity.
        min_scaled: i64,
    },
    /// Quantity exceeds the instrument maximum.
    QuantityAboveMax {
        /// Scaled quantity.
        scaled: i64,
        /// Maximum scaled quantity.
        max_scaled: i64,
    },
    /// Price falls outside all tick bands.
    PriceOutOfBands {
        /// Scaled price.
        scaled: i64,
    },
    /// Instrument is not open for new orders.
    NotTradable {
        /// Instrument that rejected trading.
        id: InstrumentId,
    },
    /// Alias did not resolve to an instrument.
    UnknownAlias,
    /// Internal id was not found in the master.
    UnknownInstrument {
        /// Missing id.
        id: InstrumentId,
    },
    /// Alias is already bound to a different instrument.
    DuplicateAlias {
        /// Conflicting instrument already registered.
        existing: InstrumentId,
    },
    /// Instrument id is already present in the master.
    DuplicateInstrument {
        /// Conflicting id.
        id: InstrumentId,
    },
}

impl fmt::Display for InstrumentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidIdentifier => f.write_str("invalid identifier"),
            Self::InvalidIsin => f.write_str("invalid ISIN"),
            Self::InvalidMic => f.write_str("invalid MIC"),
            Self::ScaleOutOfRange { scale } => {
                write!(f, "scale {scale} exceeds maximum")
            }
            Self::InvalidGrid(msg) => write!(f, "invalid trading grid: {msg}"),
            Self::InvalidDecimal => f.write_str("invalid decimal"),
            Self::ScaleExceeded { digits, scale } => {
                write!(f, "fractional digits {digits} exceed scale {scale}")
            }
            Self::Overflow => f.write_str("arithmetic overflow"),
            Self::PriceOffGrid {
                scaled,
                tick_size_scaled,
            } => write!(
                f,
                "price {scaled} is not a multiple of tick {tick_size_scaled}"
            ),
            Self::QuantityOffGrid {
                scaled,
                step_scaled,
            } => write!(
                f,
                "quantity {scaled} is not a multiple of step {step_scaled}"
            ),
            Self::QuantityBelowMin { scaled, min_scaled } => {
                write!(f, "quantity {scaled} below minimum {min_scaled}")
            }
            Self::QuantityAboveMax { scaled, max_scaled } => {
                write!(f, "quantity {scaled} above maximum {max_scaled}")
            }
            Self::PriceOutOfBands { scaled } => {
                write!(f, "price {scaled} outside tick bands")
            }
            Self::NotTradable { id } => write!(f, "instrument {id} is not tradable"),
            Self::UnknownAlias => f.write_str("unknown instrument alias"),
            Self::UnknownInstrument { id } => write!(f, "unknown instrument {id}"),
            Self::DuplicateAlias { existing } => {
                write!(f, "alias already bound to instrument {existing}")
            }
            Self::DuplicateInstrument { id } => {
                write!(f, "duplicate instrument id {id}")
            }
        }
    }
}

impl std::error::Error for InstrumentError {}
