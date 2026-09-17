//! In-memory pre-trade risk engine (rebuildable from events).

use std::collections::HashSet;

use shinrai_instruments::{InstrumentId, PriceTicks, QuantityLots};
use shinrai_ledger::AccountId;
use shinrai_money::Money;
use shinrai_orders::Side;

use crate::limits::RiskLimits;
use crate::reason::RiskRejectReason;

/// Outcome of a pre-trade check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RiskDecision {
    /// Order may proceed to the OMS.
    Approved,
    /// Order must not reach the OMS.
    Rejected(RiskRejectReason),
}

impl RiskDecision {
    /// Returns the rejection reason when rejected.
    #[must_use]
    pub const fn reject_reason(self) -> Option<RiskRejectReason> {
        match self {
            Self::Approved => None,
            Self::Rejected(r) => Some(r),
        }
    }
}

/// Snapshot of account state needed for a check (no I/O).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RiskContext {
    /// Unreserved cash in the quote currency.
    pub available_cash: Money,
    /// Current signed position in lots.
    pub position_lots: i64,
    /// Order notional in quote currency.
    pub notional: Money,
    /// Optional reference / mark price for collar checks.
    pub ref_price: Option<PriceTicks>,
    /// Logical unix seconds (for market hours).
    pub now_unix: u64,
    /// Realized + unrealized day P&L in minor units (negative = loss).
    pub day_pnl_minor: i128,
    /// Existing notional exposure for this asset class (absolute minor units).
    pub asset_class_exposure_minor: i128,
}

/// Order intent presented to risk (before OMS mutation).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RiskOrderIntent {
    /// Account submitting the order.
    pub account_id: AccountId,
    /// Instrument.
    pub instrument_id: InstrumentId,
    /// Side.
    pub side: Side,
    /// Quantity in lots.
    pub qty: QuantityLots,
    /// Limit price in ticks.
    pub price: PriceTicks,
}

/// Pre-trade risk state and limits.
#[derive(Debug, Clone)]
pub struct RiskEngine {
    limits: RiskLimits,
    global_kill: bool,
    account_kills: HashSet<AccountId>,
    restricted: HashSet<InstrumentId>,
}

impl Default for RiskEngine {
    fn default() -> Self {
        Self::new(RiskLimits::default())
    }
}

impl RiskEngine {
    /// Creates an engine with the given limits.
    #[must_use]
    pub fn new(limits: RiskLimits) -> Self {
        Self {
            limits,
            global_kill: false,
            account_kills: HashSet::new(),
            restricted: HashSet::new(),
        }
    }

    /// Current limits.
    #[must_use]
    pub const fn limits(&self) -> RiskLimits {
        self.limits
    }

    /// Replaces limits (ops control plane).
    pub fn set_limits(&mut self, limits: RiskLimits) {
        self.limits = limits;
    }

    /// Whether the global kill switch is on.
    #[must_use]
    pub const fn global_kill(&self) -> bool {
        self.global_kill
    }

    /// Sets the global kill switch (fail closed when enabled).
    pub fn set_global_kill(&mut self, on: bool) {
        self.global_kill = on;
    }

    /// Enables or disables kill switch for one account.
    pub fn set_account_kill(&mut self, account: AccountId, on: bool) {
        if on {
            self.account_kills.insert(account);
        } else {
            self.account_kills.remove(&account);
        }
    }

    /// Marks an instrument as restricted.
    pub fn restrict_instrument(&mut self, id: InstrumentId) {
        self.restricted.insert(id);
    }

    /// Clears instrument restriction.
    pub fn allow_instrument(&mut self, id: InstrumentId) {
        self.restricted.remove(&id);
    }

    /// Runs pre-trade checks. Does not mutate ledger or OMS state.
    #[must_use]
    #[allow(clippy::too_many_lines)]
    pub fn check(&self, intent: &RiskOrderIntent, ctx: &RiskContext) -> RiskDecision {
        if self.global_kill || self.account_kills.contains(&intent.account_id) {
            return RiskDecision::Rejected(RiskRejectReason::KillSwitch);
        }
        if self.restricted.contains(&intent.instrument_id) {
            return RiskDecision::Rejected(RiskRejectReason::RestrictedInstrument);
        }
        if let Some((open, close)) = self.limits.market_session_utc {
            let tod = (ctx.now_unix % 86_400) as u32;
            let in_session = if open <= close {
                tod >= open && tod < close
            } else {
                tod >= open || tod < close
            };
            if !in_session {
                return RiskDecision::Rejected(RiskRejectReason::MarketClosed);
            }
        }
        if self.limits.max_daily_loss_minor > 0
            && ctx.day_pnl_minor <= -self.limits.max_daily_loss_minor
        {
            return RiskDecision::Rejected(RiskRejectReason::DailyLossLimit);
        }
        let qty = intent.qty.lots();
        if qty <= 0 || qty > self.limits.max_order_qty_lots {
            return RiskDecision::Rejected(RiskRejectReason::MaxQuantity);
        }
        if ctx.notional.minor_units() <= 0
            || ctx.notional.minor_units() > self.limits.max_order_notional_minor
        {
            return RiskDecision::Rejected(RiskRejectReason::MaxNotional);
        }
        if self.limits.collar_bps > 0 {
            if let Some(ref_px) = ctx.ref_price {
                let limit = intent.price.scaled();
                let mark = ref_px.scaled();
                if mark > 0 {
                    let diff = i128::from((limit - mark).unsigned_abs());
                    let max_diff = (i128::from(mark) * i128::from(self.limits.collar_bps)) / 10_000;
                    if diff > max_diff {
                        return RiskDecision::Rejected(RiskRejectReason::PriceCollar);
                    }
                }
            }
        }
        if self.limits.max_asset_class_notional_minor > 0 {
            let add =
                i128::try_from(ctx.notional.minor_units().unsigned_abs()).unwrap_or(i128::MAX);
            let projected = ctx.asset_class_exposure_minor.saturating_add(add);
            if projected > self.limits.max_asset_class_notional_minor {
                return RiskDecision::Rejected(RiskRejectReason::MaxAssetClassExposure);
            }
        }
        match intent.side {
            Side::Buy => {
                if ctx.available_cash.currency() != ctx.notional.currency()
                    || ctx.available_cash.minor_units() < ctx.notional.minor_units()
                {
                    return RiskDecision::Rejected(RiskRejectReason::InsufficientBuyingPower);
                }
                let new_pos = ctx.position_lots.saturating_add(qty);
                if new_pos > self.limits.max_position_lots {
                    return RiskDecision::Rejected(RiskRejectReason::MaxPosition);
                }
            }
            Side::Sell => {
                if ctx.position_lots < qty {
                    if !self.limits.allow_short {
                        return RiskDecision::Rejected(RiskRejectReason::InsufficientPosition);
                    }
                    let new_short = qty - ctx.position_lots;
                    if new_short > self.limits.max_short_lots {
                        return RiskDecision::Rejected(RiskRejectReason::MaxShort);
                    }
                }
            }
        }
        RiskDecision::Approved
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shinrai_money::Currency;

    fn ctx(cash: i128, pos: i64, notional: i128) -> RiskContext {
        RiskContext {
            available_cash: Money::from_minor(cash, Currency::usd()),
            position_lots: pos,
            notional: Money::from_minor(notional, Currency::usd()),
            ref_price: None,
            now_unix: 0,
            day_pnl_minor: 0,
            asset_class_exposure_minor: 0,
        }
    }

    fn buy(qty: i64, px: i64) -> RiskOrderIntent {
        RiskOrderIntent {
            account_id: AccountId::from_u64(1),
            instrument_id: InstrumentId::from_u64(1),
            side: Side::Buy,
            qty: QuantityLots::from_lots(qty),
            price: PriceTicks::from_scaled(px),
        }
    }

    #[test]
    fn kill_switch_blocks() {
        let mut engine = RiskEngine::new(RiskLimits::demo());
        engine.set_global_kill(true);
        assert_eq!(
            engine
                .check(&buy(1, 100), &ctx(1_000_000, 0, 100))
                .reject_reason(),
            Some(RiskRejectReason::KillSwitch)
        );
    }

    #[test]
    fn collar_rejects_far_limit() {
        let mut limits = RiskLimits::demo();
        limits.collar_bps = 100; // 1%
        let engine = RiskEngine::new(limits);
        let mut c = ctx(10_000_000, 0, 10_000);
        c.ref_price = Some(PriceTicks::from_scaled(10_000));
        assert_eq!(
            engine.check(&buy(1, 10_500), &c).reject_reason(),
            Some(RiskRejectReason::PriceCollar)
        );
    }

    #[test]
    fn market_hours_reject() {
        let mut limits = RiskLimits::demo();
        limits.market_session_utc = Some((14 * 3600, 21 * 3600));
        let engine = RiskEngine::new(limits);
        let mut c = ctx(10_000_000, 0, 100);
        c.now_unix = 10 * 3600; // 10:00 UTC
        assert_eq!(
            engine.check(&buy(1, 100), &c).reject_reason(),
            Some(RiskRejectReason::MarketClosed)
        );
    }

    #[test]
    fn daily_loss_and_short() {
        let mut limits = RiskLimits::demo();
        limits.max_daily_loss_minor = 1_000;
        limits.allow_short = true;
        limits.max_short_lots = 5;
        let engine = RiskEngine::new(limits);
        let mut c = ctx(10_000_000, 0, 100);
        c.day_pnl_minor = -2_000;
        assert_eq!(
            engine.check(&buy(1, 100), &c).reject_reason(),
            Some(RiskRejectReason::DailyLossLimit)
        );
        c.day_pnl_minor = 0;
        let sell = RiskOrderIntent {
            account_id: AccountId::from_u64(1),
            instrument_id: InstrumentId::from_u64(1),
            side: Side::Sell,
            qty: QuantityLots::from_lots(3),
            price: PriceTicks::from_scaled(100),
        };
        assert_eq!(engine.check(&sell, &c), RiskDecision::Approved);
        let sell_big = RiskOrderIntent {
            qty: QuantityLots::from_lots(10),
            ..sell
        };
        assert_eq!(
            engine.check(&sell_big, &c).reject_reason(),
            Some(RiskRejectReason::MaxShort)
        );
    }
}
