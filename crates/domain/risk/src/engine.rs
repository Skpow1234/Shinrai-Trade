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
    pub fn check(&self, intent: &RiskOrderIntent, ctx: &RiskContext) -> RiskDecision {
        if self.global_kill || self.account_kills.contains(&intent.account_id) {
            return RiskDecision::Rejected(RiskRejectReason::KillSwitch);
        }
        if self.restricted.contains(&intent.instrument_id) {
            return RiskDecision::Rejected(RiskRejectReason::RestrictedInstrument);
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
                    return RiskDecision::Rejected(RiskRejectReason::InsufficientPosition);
                }
            }
        }
        RiskDecision::Approved
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shinrai_instruments::{aapl, PriceTicks, QuantityLots};
    use shinrai_money::{Currency, Money};

    fn ctx(notional_minor: i128, available_minor: i128, position: i64) -> RiskContext {
        let currency = Currency::usd();
        RiskContext {
            available_cash: Money::from_minor(available_minor, currency),
            position_lots: position,
            notional: Money::from_minor(notional_minor, currency),
        }
    }

    fn buy_intent(qty: i64) -> RiskOrderIntent {
        RiskOrderIntent {
            account_id: AccountId::from_u64(1),
            instrument_id: aapl().id(),
            side: Side::Buy,
            qty: QuantityLots::from_lots(qty),
            price: PriceTicks::from_scaled(10_000),
        }
    }

    #[test]
    fn approves_when_within_limits() {
        let engine = RiskEngine::default();
        assert_eq!(
            engine.check(&buy_intent(10), &ctx(100_000, 200_000, 0)),
            RiskDecision::Approved
        );
    }

    #[test]
    fn rejects_insufficient_buying_power() {
        let engine = RiskEngine::default();
        assert_eq!(
            engine
                .check(&buy_intent(10), &ctx(100_000, 50_000, 0))
                .reject_reason(),
            Some(RiskRejectReason::InsufficientBuyingPower)
        );
    }

    #[test]
    fn kill_switch_blocks() {
        let mut engine = RiskEngine::default();
        engine.set_global_kill(true);
        assert_eq!(
            engine
                .check(&buy_intent(1), &ctx(100, 1_000, 0))
                .reject_reason(),
            Some(RiskRejectReason::KillSwitch)
        );
    }

    fn sell_intent(qty: i64) -> RiskOrderIntent {
        RiskOrderIntent {
            account_id: AccountId::from_u64(1),
            instrument_id: aapl().id(),
            side: Side::Sell,
            qty: QuantityLots::from_lots(qty),
            price: PriceTicks::from_scaled(10_000),
        }
    }

    #[test]
    fn rejects_insufficient_position_on_sell() {
        let engine = RiskEngine::default();
        assert_eq!(
            engine
                .check(&sell_intent(5), &ctx(50_000, 1_000_000, 2))
                .reject_reason(),
            Some(RiskRejectReason::InsufficientPosition)
        );
    }

    #[test]
    fn approves_sell_within_position() {
        let engine = RiskEngine::default();
        assert_eq!(
            engine.check(&sell_intent(5), &ctx(50_000, 1_000_000, 10)),
            RiskDecision::Approved
        );
    }

    #[test]
    fn restricted_instrument_blocks() {
        let mut engine = RiskEngine::default();
        engine.restrict_instrument(aapl().id());
        assert_eq!(
            engine
                .check(&buy_intent(1), &ctx(100, 1_000, 0))
                .reject_reason(),
            Some(RiskRejectReason::RestrictedInstrument)
        );
    }
}
