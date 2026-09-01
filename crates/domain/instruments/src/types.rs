//! Classification and lifecycle status enums.

/// Broad asset class for an instrument.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AssetClass {
    /// Listed equity.
    Equity,
    /// Exchange-traded future.
    Future,
    /// Crypto spot pair.
    Crypto,
    /// Other / unclassified.
    Other,
}

/// More specific instrument type within an [`AssetClass`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InstrumentType {
    /// Common stock.
    CommonStock,
    /// Index future.
    IndexFuture,
    /// Crypto spot.
    CryptoSpot,
    /// Other.
    Other,
}

/// Trading status of an instrument.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InstrumentStatus {
    /// Open for trading.
    Active,
    /// Temporarily halted.
    Halted,
    /// Permanently delisted.
    Delisted,
    /// Expired (derivatives).
    Expired,
}

impl InstrumentStatus {
    /// Returns true if new orders may be accepted.
    #[must_use]
    pub const fn is_tradable(self) -> bool {
        matches!(self, Self::Active)
    }
}
