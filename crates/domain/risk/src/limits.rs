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
}

impl Default for RiskLimits {
    /// Permissive defaults so Phase 1 paper tests stay unchanged.
    fn default() -> Self {
        Self {
            max_order_qty_lots: 1_000_000,
            max_order_notional_minor: i128::from(i64::MAX),
            max_position_lots: 1_000_000,
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
        }
    }
}
