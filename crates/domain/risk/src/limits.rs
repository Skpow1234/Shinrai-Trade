//! Configurable risk limits.

/// Static limits applied on every order check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RiskLimits {
    /// Maximum order quantity in lots.
    pub max_order_qty_lots: i64,
    /// Maximum order notional in quote-currency minor units.
    pub max_order_notional_minor: i128,
    /// Maximum absolute position in lots after the fill (buy adds lots).
    pub max_position_lots: i64,
    /// Max absolute deviation from reference price in basis points (`0` = off).
    pub collar_bps: i64,
    /// Market session as (`open_secs`, `close_secs`) from midnight UTC; `None` = always open.
    pub market_session_utc: Option<(u32, u32)>,
    /// Max absolute day P&L loss in minor units (`0` = off). Reject when `day_pnl <= -limit`.
    pub max_daily_loss_minor: i128,
    /// When false, sells may not exceed long position (no shorts).
    pub allow_short: bool,
    /// Max short position in lots when shorts are allowed.
    pub max_short_lots: i64,
    /// Max notional exposure for the order's asset class (`0` = off).
    pub max_asset_class_notional_minor: i128,
}

impl Default for RiskLimits {
    /// Permissive defaults so Phase 1 paper tests stay unchanged.
    fn default() -> Self {
        Self {
            max_order_qty_lots: 1_000_000,
            max_order_notional_minor: i128::from(i64::MAX),
            max_position_lots: 1_000_000,
            collar_bps: 0,
            market_session_utc: None,
            max_daily_loss_minor: 0,
            allow_short: false,
            max_short_lots: 0,
            max_asset_class_notional_minor: 0,
        }
    }
}

impl RiskLimits {
    /// Tight limits for gateway demos and tests.
    #[must_use]
    pub const fn demo() -> Self {
        Self {
            max_order_qty_lots: 10_000,
            max_order_notional_minor: 50_000_000, // 500_000 USD at 2dp
            max_position_lots: 100_000,
            collar_bps: 0,
            market_session_utc: None,
            max_daily_loss_minor: 0,
            allow_short: false,
            max_short_lots: 0,
            max_asset_class_notional_minor: 0,
        }
    }
}
