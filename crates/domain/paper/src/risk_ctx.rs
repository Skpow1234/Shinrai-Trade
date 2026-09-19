//! Pre-trade risk context helpers (day P&L + asset-class exposure).
//!
//! Day boundary is UTC midnight of `logical_now` (`now_unix / 86_400`).
//! Realized is average-cost (same semantics as portfolio); unrealized uses
//! in-engine fill marks when present.

use std::collections::HashMap;

use shinrai_instruments::{
    AssetClass, Instrument, InstrumentId, InstrumentMaster, PriceTicks, QuantityLots,
};
use shinrai_ledger::{AccountId, PaperBook};
use shinrai_money::MoneyError;
use shinrai_orders::{OrderStore, Side};

use crate::notional::notional;
use crate::PaperError;

/// Seconds in a UTC calendar day.
pub const SECS_PER_DAY: u64 = 86_400;

/// UTC day id (`unix / 86400`) and midnight start for that day.
#[must_use]
pub fn utc_day_id(now_unix: u64) -> u64 {
    now_unix / SECS_PER_DAY
}

/// UTC midnight unix seconds for the day containing `now_unix`.
#[must_use]
pub fn utc_day_start(now_unix: u64) -> u64 {
    utc_day_id(now_unix).saturating_mul(SECS_PER_DAY)
}

#[derive(Debug, Clone, Copy)]
struct InventoryLot {
    lots: i64,
    cost_minor: i128,
    avg_scaled: i64,
}

/// All-time average-cost realized P&L for an account (minor units).
///
/// # Errors
///
/// Returns [`PaperError::Money`] / instrument lookup failures as money overflow.
pub fn realized_pnl_all_time(
    account: AccountId,
    orders: &OrderStore,
    master: &InstrumentMaster,
) -> Result<i128, PaperError> {
    inventory_from_orders(account, orders, master).map(|(r, _)| r)
}

/// Unrealized MTM P&L using marks (0 when no markable positions).
///
/// # Errors
///
/// Returns money overflow from notional math.
pub fn unrealized_pnl_minor(
    account: AccountId,
    book: &PaperBook,
    orders: &OrderStore,
    master: &InstrumentMaster,
    marks: &HashMap<InstrumentId, PriceTicks>,
) -> Result<i128, PaperError> {
    let (_, inv) = inventory_from_orders(account, orders, master)?;
    let mut total = 0i128;
    for (inst_id, lots) in book.positions_for(account) {
        if lots == 0 {
            continue;
        }
        let Some(mark) = marks.get(&inst_id).copied() else {
            continue;
        };
        let Some(lot) = inv.get(&inst_id) else {
            continue;
        };
        if lot.lots == 0 {
            continue;
        }
        let instrument = master.get(inst_id)?;
        let abs_lots = lots.unsigned_abs();
        let qty = QuantityLots::from_lots(i64::try_from(abs_lots).unwrap_or(i64::MAX));
        let mv = notional(instrument, mark, qty)?.minor_units();
        let cost = if lots > 0 {
            lot.cost_minor
                .checked_mul(i128::from(lots))
                .and_then(|v| v.checked_div(i128::from(lot.lots)))
                .ok_or(MoneyError::Overflow)?
        } else {
            // Short: use avg cost × abs lots
            i128::from(lot.avg_scaled)
                .checked_mul(i128::from(lots.abs()))
                .ok_or(MoneyError::Overflow)?
        };
        let pnl = if lots > 0 {
            mv.checked_sub(cost).ok_or(MoneyError::Overflow)?
        } else {
            cost.checked_sub(mv).ok_or(MoneyError::Overflow)?
        };
        total = total.checked_add(pnl).ok_or(MoneyError::Overflow)?;
    }
    Ok(total)
}

/// Absolute notional exposure for open positions in `asset_class` (minor units).
///
/// Prefers fill marks; falls back to average cost from inventory. Does **not**
/// include the prospective order notional (risk engine adds that).
///
/// # Errors
///
/// Returns money / instrument errors.
pub fn asset_class_exposure_minor(
    account: AccountId,
    book: &PaperBook,
    orders: &OrderStore,
    master: &InstrumentMaster,
    marks: &HashMap<InstrumentId, PriceTicks>,
    asset_class: AssetClass,
) -> Result<i128, PaperError> {
    let (_, inv) = inventory_from_orders(account, orders, master)?;
    let mut exposure = 0i128;
    for (inst_id, lots) in book.positions_for(account) {
        if lots == 0 {
            continue;
        }
        let instrument = master.get(inst_id)?;
        if instrument.asset_class() != asset_class {
            continue;
        }
        let px = marks
            .get(&inst_id)
            .copied()
            .or_else(|| {
                inv.get(&inst_id)
                    .map(|lot| PriceTicks::from_scaled(lot.avg_scaled))
            })
            .unwrap_or_else(|| PriceTicks::from_scaled(0));
        if px.scaled() <= 0 {
            continue;
        }
        let abs_lots = i64::try_from(lots.unsigned_abs()).unwrap_or(i64::MAX);
        let n = notional(instrument, px, QuantityLots::from_lots(abs_lots))?;
        let add = i128::try_from(n.minor_units().unsigned_abs()).unwrap_or(i128::MAX);
        exposure = exposure.checked_add(add).ok_or(MoneyError::Overflow)?;
    }
    Ok(exposure)
}

/// Seeds marks from filled orders' average / limit prices.
pub fn seed_marks_from_orders(orders: &OrderStore, marks: &mut HashMap<InstrumentId, PriceTicks>) {
    for order in orders.orders() {
        if order.cum_qty().lots() <= 0 {
            continue;
        }
        let px = order.avg_px().unwrap_or_else(|| order.price());
        if px.scaled() > 0 {
            marks.insert(order.instrument_id(), px);
        }
    }
}

fn inventory_from_orders(
    account: AccountId,
    orders: &OrderStore,
    master: &InstrumentMaster,
) -> Result<(i128, HashMap<InstrumentId, InventoryLot>), PaperError> {
    let mut realized = 0i128;
    let mut by_inst: HashMap<InstrumentId, InventoryLot> = HashMap::new();
    let mut filled: Vec<_> = orders
        .orders()
        .filter(|o| o.account_id() == account && o.cum_qty().lots() > 0)
        .collect();
    filled.sort_by_key(|o| o.id().get());

    for order in filled {
        let inst = order.instrument_id();
        let instrument = master.get(inst)?;
        let qty = order.cum_qty().lots();
        let px = order.avg_px().unwrap_or_else(|| order.price());
        match order.side() {
            Side::Buy => {
                let fill_cost = notional_for(instrument, px, QuantityLots::from_lots(qty))?;
                let entry = by_inst.entry(inst).or_insert(InventoryLot {
                    lots: 0,
                    cost_minor: 0,
                    avg_scaled: px.scaled(),
                });
                entry.avg_scaled =
                    weighted_avg_ticks(entry.avg_scaled, entry.lots, px.scaled(), qty);
                entry.lots = entry.lots.checked_add(qty).ok_or(MoneyError::Overflow)?;
                entry.cost_minor = entry
                    .cost_minor
                    .checked_add(fill_cost.minor_units())
                    .ok_or(MoneyError::Overflow)?;
            }
            Side::Sell => {
                let Some(entry) = by_inst.get_mut(&inst) else {
                    continue;
                };
                if entry.lots < qty {
                    continue;
                }
                let cost_for_sale = entry
                    .cost_minor
                    .checked_mul(i128::from(qty))
                    .and_then(|v| v.checked_div(i128::from(entry.lots)))
                    .ok_or(MoneyError::Overflow)?;
                let proceeds = notional_for(instrument, px, QuantityLots::from_lots(qty))?;
                realized = realized
                    .checked_add(
                        proceeds
                            .minor_units()
                            .checked_sub(cost_for_sale)
                            .ok_or(MoneyError::Overflow)?,
                    )
                    .ok_or(MoneyError::Overflow)?;
                entry.lots -= qty;
                entry.cost_minor -= cost_for_sale;
                if entry.lots == 0 {
                    entry.cost_minor = 0;
                }
            }
        }
    }
    Ok((realized, by_inst))
}

fn notional_for(
    instrument: &Instrument,
    price: PriceTicks,
    qty: QuantityLots,
) -> Result<shinrai_money::Money, PaperError> {
    notional(instrument, price, qty)
}

fn weighted_avg_ticks(prev_avg: i64, prev_lots: i64, px: i64, qty: i64) -> i64 {
    let total_lots = prev_lots + qty;
    if total_lots == 0 {
        return px;
    }
    let blended = (i128::from(prev_avg) * i128::from(prev_lots) + i128::from(px) * i128::from(qty))
        / i128::from(total_lots);
    i64::try_from(blended).unwrap_or(prev_avg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use shinrai_exchange_simulator::FaultConfig;
    use shinrai_instruments::{aapl, phase1_master};
    use shinrai_money::{Currency, Money};
    use shinrai_orders::{ClientOrderId, OrderType, TimeInForce};
    use shinrai_risk::{RiskEngine, RiskLimits};

    use crate::{PaperEngine, SubmitRequest};

    fn buy(engine: &mut PaperEngine, acc: AccountId, clid: &str, qty: i64, px: i64) {
        engine
            .submit(&SubmitRequest {
                account_id: acc,
                client_order_id: ClientOrderId::new(clid).expect("c"),
                instrument_id: aapl().id(),
                side: Side::Buy,
                qty: QuantityLots::from_lots(qty),
                price: PriceTicks::from_scaled(px),
                order_type: OrderType::Limit,
                time_in_force: TimeInForce::Gtc,
            })
            .expect("buy");
    }

    #[test]
    fn utc_day_helpers() {
        assert_eq!(utc_day_start(86_400), 86_400);
        assert_eq!(utc_day_id(86_399), 0);
        assert_eq!(utc_day_id(86_400), 1);
    }

    #[test]
    fn exposure_same_class_only() {
        let mut engine = PaperEngine::new(phase1_master(), FaultConfig::happy_path());
        let acc = AccountId::from_u64(1);
        engine
            .deposit(
                acc,
                Money::from_major(100_000, Currency::usd()).expect("d"),
                "dep",
            )
            .expect("dep");
        buy(&mut engine, acc, "e1", 10, 10_000);

        let equity_exp = asset_class_exposure_minor(
            acc,
            engine.book(),
            engine.orders(),
            engine.master(),
            engine.marks(),
            AssetClass::Equity,
        )
        .expect("exp");
        assert_eq!(equity_exp, 100_000); // 10 * $100.00
        let crypto_exp = asset_class_exposure_minor(
            acc,
            engine.book(),
            engine.orders(),
            engine.master(),
            engine.marks(),
            AssetClass::Crypto,
        )
        .expect("crypto");
        assert_eq!(crypto_exp, 0);
    }

    #[test]
    fn daily_loss_blocks_after_losing_sell() {
        let mut limits = RiskLimits::demo();
        limits.max_daily_loss_minor = 500; // $5 at 2dp
        let mut engine = PaperEngine::with_risk(
            phase1_master(),
            FaultConfig::happy_path(),
            RiskEngine::new(limits),
        );
        let acc = AccountId::from_u64(1);
        engine
            .deposit(
                acc,
                Money::from_major(10_000, Currency::usd()).expect("d"),
                "dep",
            )
            .expect("dep");
        engine.set_logical_now(86_400); // day 1
        buy(&mut engine, acc, "b1", 10, 10_000);
        engine
            .submit(&SubmitRequest {
                account_id: acc,
                client_order_id: ClientOrderId::new("s1").expect("c"),
                instrument_id: aapl().id(),
                side: Side::Sell,
                qty: QuantityLots::from_lots(10),
                price: PriceTicks::from_scaled(9_900), // lose $10
                order_type: OrderType::Limit,
                time_in_force: TimeInForce::Gtc,
            })
            .expect("sell");

        let err = engine
            .submit(&SubmitRequest {
                account_id: acc,
                client_order_id: ClientOrderId::new("b2").expect("c"),
                instrument_id: aapl().id(),
                side: Side::Buy,
                qty: QuantityLots::from_lots(1),
                price: PriceTicks::from_scaled(10_000),
                order_type: OrderType::Limit,
                time_in_force: TimeInForce::Gtc,
            })
            .expect_err("daily loss");
        assert!(matches!(
            err,
            PaperError::Risk(shinrai_risk::RiskRejectReason::DailyLossLimit)
        ));
    }

    #[test]
    fn day_rollover_resets_daily_loss() {
        let mut limits = RiskLimits::demo();
        limits.max_daily_loss_minor = 500;
        let mut engine = PaperEngine::with_risk(
            phase1_master(),
            FaultConfig::happy_path(),
            RiskEngine::new(limits),
        );
        let acc = AccountId::from_u64(1);
        engine
            .deposit(
                acc,
                Money::from_major(10_000, Currency::usd()).expect("d"),
                "dep",
            )
            .expect("dep");
        engine.set_logical_now(86_400);
        buy(&mut engine, acc, "b1", 10, 10_000);
        engine
            .submit(&SubmitRequest {
                account_id: acc,
                client_order_id: ClientOrderId::new("s1").expect("c"),
                instrument_id: aapl().id(),
                side: Side::Sell,
                qty: QuantityLots::from_lots(10),
                price: PriceTicks::from_scaled(9_900),
                order_type: OrderType::Limit,
                time_in_force: TimeInForce::Gtc,
            })
            .expect("sell");

        engine.set_logical_now(86_400 * 2); // next UTC day
        engine
            .submit(&SubmitRequest {
                account_id: acc,
                client_order_id: ClientOrderId::new("b2").expect("c"),
                instrument_id: aapl().id(),
                side: Side::Buy,
                qty: QuantityLots::from_lots(1),
                price: PriceTicks::from_scaled(10_000),
                order_type: OrderType::Limit,
                time_in_force: TimeInForce::Gtc,
            })
            .expect("new day allows trade");
    }

    #[test]
    fn asset_class_cap_rejects_same_class() {
        let mut limits = RiskLimits::demo();
        limits.max_asset_class_notional_minor = 105_000; // first $1000 buy ok; +$100 trips
        let mut engine = PaperEngine::with_risk(
            phase1_master(),
            FaultConfig::happy_path(),
            RiskEngine::new(limits),
        );
        let acc = AccountId::from_u64(1);
        engine
            .deposit(
                acc,
                Money::from_major(10_000, Currency::usd()).expect("d"),
                "dep",
            )
            .expect("dep");
        buy(&mut engine, acc, "b1", 10, 10_000); // $1000 exposure
        let err = engine
            .submit(&SubmitRequest {
                account_id: acc,
                client_order_id: ClientOrderId::new("b2").expect("c"),
                instrument_id: aapl().id(),
                side: Side::Buy,
                qty: QuantityLots::from_lots(1),
                price: PriceTicks::from_scaled(10_000),
                order_type: OrderType::Limit,
                time_in_force: TimeInForce::Gtc,
            })
            .expect_err("cap");
        assert!(matches!(
            err,
            PaperError::Risk(shinrai_risk::RiskRejectReason::MaxAssetClassExposure)
        ));
    }
}
