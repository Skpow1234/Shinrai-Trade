//! Provider and decode errors.

use core::fmt;

use shinrai_instruments::InstrumentError;
use shinrai_market_data::MdError;

/// Errors from vendor decode or snapshot handling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderError {
    /// Payload is not JSON.
    InvalidJson,
    /// Required field missing or wrong type.
    MissingField(&'static str),
    /// Vendor product id is not in the instrument master.
    UnknownInstrument {
        /// Vendor symbol (e.g. `BTC-USD`).
        product_id: String,
    },
    /// Instrument grid rejected the price.
    Instrument(InstrumentError),
    /// Normalized record failed validation.
    MarketData(MdError),
}

impl fmt::Display for ProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidJson => f.write_str("invalid vendor JSON"),
            Self::MissingField(name) => write!(f, "missing field {name}"),
            Self::UnknownInstrument { product_id } => {
                write!(f, "unknown vendor instrument {product_id}")
            }
            Self::Instrument(e) => write!(f, "{e}"),
            Self::MarketData(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ProviderError {}

impl From<InstrumentError> for ProviderError {
    fn from(value: InstrumentError) -> Self {
        Self::Instrument(value)
    }
}

impl From<MdError> for ProviderError {
    fn from(value: MdError) -> Self {
        Self::MarketData(value)
    }
}
