//! Stable instrument identity and external aliases.

use core::fmt;

use crate::error::InstrumentError;

/// Opaque internal instrument identifier.
///
/// Vendor symbols change and collide across venues; this id does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct InstrumentId(u64);

impl InstrumentId {
    /// Creates an instrument id from a raw `u64`.
    #[must_use]
    pub const fn from_u64(id: u64) -> Self {
        Self(id)
    }

    /// Returns the raw id.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for InstrumentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Kind of external market identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IdType {
    /// ISO 6166 ISIN.
    Isin,
    /// FIGI.
    Figi,
    /// CUSIP.
    Cusip,
    /// Exchange ticker (optionally scoped by MIC via [`ExternalId::venue`]).
    Ticker,
    /// Broker / vendor symbol.
    BrokerSymbol,
    /// Other / custom.
    Other,
}

/// An external identifier that can resolve to an [`InstrumentId`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExternalId {
    id_type: IdType,
    value: String,
    venue: Option<String>,
}

impl ExternalId {
    /// Creates an external id.
    ///
    /// # Errors
    ///
    /// Returns [`InstrumentError::InvalidIdentifier`] if `value` is empty after trim.
    pub fn new(
        id_type: IdType,
        value: impl Into<String>,
        venue: Option<String>,
    ) -> Result<Self, InstrumentError> {
        let value = value.into().trim().to_owned();
        if value.is_empty() {
            return Err(InstrumentError::InvalidIdentifier);
        }
        let venue = match venue {
            Some(v) => {
                let v = v.trim().to_owned();
                if v.is_empty() {
                    None
                } else {
                    Some(v.to_ascii_uppercase())
                }
            }
            None => None,
        };
        Ok(Self {
            id_type,
            value: normalize_value(id_type, value),
            venue,
        })
    }

    /// Convenience: ticker without a venue.
    ///
    /// # Errors
    ///
    /// Returns [`InstrumentError::InvalidIdentifier`] if `ticker` is empty.
    pub fn ticker(ticker: impl Into<String>) -> Result<Self, InstrumentError> {
        Self::new(IdType::Ticker, ticker, None)
    }

    /// Convenience: ticker scoped to a MIC / venue code.
    ///
    /// # Errors
    ///
    /// Returns an error if either field is invalid.
    pub fn ticker_at(
        ticker: impl Into<String>,
        mic: impl Into<String>,
    ) -> Result<Self, InstrumentError> {
        Self::new(IdType::Ticker, ticker, Some(mic.into()))
    }

    /// Convenience: ISIN (validated separately via [`crate::validate_isin`] when inserting).
    ///
    /// # Errors
    ///
    /// Returns [`InstrumentError::InvalidIdentifier`] if empty.
    pub fn isin(isin: impl Into<String>) -> Result<Self, InstrumentError> {
        Self::new(IdType::Isin, isin, None)
    }

    /// Returns the identifier kind.
    #[must_use]
    pub const fn id_type(&self) -> IdType {
        self.id_type
    }

    /// Returns the identifier value.
    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }

    /// Returns the optional venue / MIC scope.
    #[must_use]
    pub fn venue(&self) -> Option<&str> {
        self.venue.as_deref()
    }
}

fn normalize_value(id_type: IdType, value: String) -> String {
    match id_type {
        IdType::Isin | IdType::Cusip | IdType::Figi | IdType::Ticker | IdType::BrokerSymbol => {
            value.to_ascii_uppercase()
        }
        IdType::Other => value,
    }
}
