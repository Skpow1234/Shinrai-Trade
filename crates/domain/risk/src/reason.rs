//! Risk rejection reasons.

use core::fmt;

/// Why a pre-trade check failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RiskRejectReason {
    /// Global or account kill switch is active.
    KillSwitch,
    /// Instrument is restricted for this account or globally.
    RestrictedInstrument,
    /// Order quantity exceeds the configured maximum.
    MaxQuantity,
    /// Order notional exceeds the configured maximum.
    MaxNotional,
    /// Resulting position would exceed the configured maximum.
    MaxPosition,
    /// Available cash is less than required notional (buy).
    InsufficientBuyingPower,
    /// Sell quantity exceeds available long position.
    InsufficientPosition,
    /// Limit price outside collar vs reference.
    PriceCollar,
    /// Outside configured market hours.
    MarketClosed,
    /// Daily loss limit breached.
    DailyLossLimit,
    /// Resulting short would exceed max short lots.
    MaxShort,
    /// Asset-class exposure would exceed the configured maximum.
    MaxAssetClassExposure,
}

impl RiskRejectReason {
    /// Stable API error code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::KillSwitch => "kill_switch",
            Self::RestrictedInstrument => "restricted_instrument",
            Self::MaxQuantity => "max_quantity",
            Self::MaxNotional => "max_notional",
            Self::MaxPosition => "max_position",
            Self::InsufficientBuyingPower => "insufficient_buying_power",
            Self::InsufficientPosition => "insufficient_position",
            Self::PriceCollar => "price_collar",
            Self::MarketClosed => "market_closed",
            Self::DailyLossLimit => "daily_loss_limit",
            Self::MaxShort => "max_short",
            Self::MaxAssetClassExposure => "max_asset_class_exposure",
        }
    }
}

impl fmt::Display for RiskRejectReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}
