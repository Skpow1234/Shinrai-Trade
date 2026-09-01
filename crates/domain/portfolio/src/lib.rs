//! Portfolio snapshot from ledger positions and order fill history.

mod marks;

use std::collections::HashMap;

use shinrai_instruments::{Instrument, InstrumentId, InstrumentMaster, PriceTicks};
use shinrai_ledger::{AccountId, PaperBook};
use shinrai_money::{Currency, Money, MoneyError};
use shinrai_orders::{OrderStore, Side};
use shinrai_paper::notional;

pub use marks::MarkStore;

/// Cash line for one currency.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CashBalance {
    /// Available (unreserved) cash.
    pub available: Money,
    /// Reserved for working orders.
    pub reserved: Money,
}

/// One open position with optional cost basis and mark.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PositionLine {
    /// Instrument.
    pub instrument_id: InstrumentId,
    /// Signed lots (positive = long).
    pub lots: i64,
    /// Volume-weighted average fill price in scaled ticks, if any fills exist.
    pub avg_cost_scaled: Option<i64>,
    /// Mark price in scaled ticks when supplied.
    pub mark_scaled: Option<i64>,
    /// Cost basis in quote minor units.
    pub cost_basis_minor: Option<i128>,
    /// Mark-to-market value in quote minor units.
    pub market_value_minor: Option<i128>,
    /// Unrealized P&L in quote minor units (`market - cost`).
    pub unrealized_pnl_minor: Option<i128>,
}

/// Account portfolio view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortfolioSnapshot {
    /// Account id.
    pub account_id: AccountId,
    /// Cash balances (Phase 3 paper: USD only in practice).
    pub cash: Vec<CashBalance>,
    /// Non-zero positions.
    pub positions: Vec<PositionLine>,
    /// Sum of unrealized P&L where marks were provided (quote minor units).
    pub total_unrealized_pnl_minor: Option<i128>,
    /// Total cost basis of open positions (quote minor units).
    pub total_cost_basis_minor: Option<i128>,
    /// Realized P&L from closed sell trades (average-cost method).
    pub realized_pnl_minor: i128,
    /// Cash + marked market value of positions when marks exist (quote minor units).
    pub total_equity_minor: Option<i128>,
}

/// Builds a portfolio snapshot from book state, orders, and optional marks.
///
/// Marks are keyed by instrument id. Missing marks leave P&L fields unset.
///
/// # Errors
///
/// Returns money overflow / inexact notional errors when computing values.
#[allow(clippy::too_many_lines)]
pub fn build_snapshot<S: std::hash::BuildHasher>(
    account: AccountId,
    book: &PaperBook,
    orders: &OrderStore,
    master: &InstrumentMaster,
    marks: &HashMap<InstrumentId, PriceTicks, S>,
) -> Result<PortfolioSnapshot, MoneyError> {
    let usd = Currency::usd();
    let cash = vec![CashBalance {
        available: book.available(account, usd),
        reserved: book.reserved(account, usd),
    }];

    let (realized_pnl_minor, inventory) = inventory_from_orders(account, orders, master)?;

    let mut by_instrument: HashMap<InstrumentId, PositionLine> = HashMap::new();

    for (inst, lots) in book_positions(book, account) {
        let inv = inventory.get(&inst);
        by_instrument.insert(
            inst,
            PositionLine {
                instrument_id: inst,
                lots,
                avg_cost_scaled: inv.map(|i| i.avg_scaled),
                mark_scaled: marks.get(&inst).map(|p| p.scaled()),
                cost_basis_minor: None,
                market_value_minor: None,
                unrealized_pnl_minor: None,
            },
        );
    }

    let mut total_unrealized: Option<i128> = None;
    let mut total_cost_basis: Option<i128> = None;
    let mut total_market_value: Option<i128> = None;
    let mut positions: Vec<PositionLine> = by_instrument.into_values().collect();
    for line in &mut positions {
        if line.lots == 0 {
            continue;
        }
        let instrument = master
            .get(line.instrument_id)
            .map_err(|_| MoneyError::Overflow)?;
        if let Some(avg_scaled) = line.avg_cost_scaled {
            let avg = PriceTicks::from_scaled(avg_scaled);
            let qty = shinrai_instruments::QuantityLots::from_lots(line.lots);
            let cost = notional_for(instrument, avg, qty)?;
            line.cost_basis_minor = Some(cost.minor_units());
            total_cost_basis = Some(
                total_cost_basis
                    .unwrap_or(0)
                    .checked_add(cost.minor_units())
                    .ok_or(MoneyError::Overflow)?,
            );
            if let Some(mark) = marks.get(&line.instrument_id) {
                line.mark_scaled = Some(mark.scaled());
                let mkt = notional_for(instrument, *mark, qty)?;
                line.market_value_minor = Some(mkt.minor_units());
                total_market_value = Some(
                    total_market_value
                        .unwrap_or(0)
                        .checked_add(mkt.minor_units())
                        .ok_or(MoneyError::Overflow)?,
                );
                let pnl = mkt
                    .minor_units()
                    .checked_sub(cost.minor_units())
                    .ok_or(MoneyError::Overflow)?;
                line.unrealized_pnl_minor = Some(pnl);
                total_unrealized = Some(
                    total_unrealized
                        .unwrap_or(0)
                        .checked_add(pnl)
                        .ok_or(MoneyError::Overflow)?,
                );
            }
        }
    }
    positions.retain(|p| p.lots != 0);
    positions.sort_by_key(|p| p.instrument_id.get());

    let cash_total = cash[0]
        .available
        .checked_add(cash[0].reserved)
        .map_err(|_| MoneyError::Overflow)?;
    let total_equity = total_market_value.map(|mv| {
        cash_total
            .minor_units()
            .checked_add(mv)
            .expect("overflow checked above")
    });

    Ok(PortfolioSnapshot {
        account_id: account,
        cash,
        positions,
        total_unrealized_pnl_minor: total_unrealized,
        total_cost_basis_minor: total_cost_basis,
        realized_pnl_minor,
        total_equity_minor: total_equity,
    })
}

/// Average-cost inventory and realized P&L from filled orders (sorted by order id).
fn inventory_from_orders(
    account: AccountId,
    orders: &OrderStore,
    master: &InstrumentMaster,
) -> Result<(i128, HashMap<InstrumentId, InventoryLot>), MoneyError> {
    let mut realized = 0i128;
    let mut by_inst: HashMap<InstrumentId, InventoryLot> = HashMap::new();
    let mut filled: Vec<_> = orders
        .orders()
        .filter(|o| o.account_id() == account && o.cum_qty().lots() > 0)
        .collect();
    filled.sort_by_key(|o| o.id().get());

    for order in filled {
        let inst = order.instrument_id();
        let instrument = master.get(inst).map_err(|_| MoneyError::Overflow)?;
        let qty = order.cum_qty().lots();
        let px = order.avg_px().unwrap_or_else(|| order.price());
        match order.side() {
            Side::Buy => {
                let fill_cost = notional_for(
                    instrument,
                    px,
                    shinrai_instruments::QuantityLots::from_lots(qty),
                )?;
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
                let proceeds = notional_for(
                    instrument,
                    px,
                    shinrai_instruments::QuantityLots::from_lots(qty),
                )?;
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

#[derive(Debug, Clone, Copy)]
struct InventoryLot {
    lots: i64,
    cost_minor: i128,
    avg_scaled: i64,
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

/// Realized P&L from sell fills using average cost.
///
/// # Errors
///
/// Returns money overflow when notionals do not fit in `i128`.
pub fn realized_pnl_from_orders(
    account: AccountId,
    orders: &OrderStore,
    master: &InstrumentMaster,
) -> Result<i128, MoneyError> {
    inventory_from_orders(account, orders, master).map(|(r, _)| r)
}

fn book_positions(book: &PaperBook, account: AccountId) -> Vec<(InstrumentId, i64)> {
    book.positions_for(account).collect()
}

fn notional_for(
    instrument: &Instrument,
    price: PriceTicks,
    qty: shinrai_instruments::QuantityLots,
) -> Result<Money, MoneyError> {
    notional(instrument, price, qty).map_err(|e| match e {
        shinrai_paper::PaperError::Money(m) => m,
        _ => MoneyError::Overflow,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use shinrai_exchange_simulator::FaultConfig;
    use shinrai_instruments::{aapl, phase1_master};
    use shinrai_orders::{ClientOrderId, SubmitOutcome};
    use shinrai_paper::{PaperEngine, SubmitRequest};

    #[test]
    fn snapshot_after_buy_shows_position_and_cash() {
        let mut engine = PaperEngine::new(phase1_master(), FaultConfig::happy_path());
        let acc = AccountId::from_u64(1);
        engine
            .deposit(
                acc,
                Money::from_major(10_000, Currency::usd()).expect("d"),
                "dep",
            )
            .expect("dep");
        let req = SubmitRequest {
            account_id: acc,
            client_order_id: ClientOrderId::new("p1").expect("c"),
            instrument_id: aapl().id(),
            side: Side::Buy,
            qty: shinrai_instruments::QuantityLots::from_lots(10),
            price: PriceTicks::from_scaled(10_000),
        };
        assert!(matches!(
            engine.submit(&req).expect("s"),
            SubmitOutcome::Created(_)
        ));

        let mut marks = HashMap::new();
        marks.insert(aapl().id(), PriceTicks::from_scaled(11_000));
        let snap = build_snapshot(
            acc,
            engine.book(),
            engine.orders(),
            &phase1_master(),
            &marks,
        )
        .expect("snap");
        assert_eq!(snap.positions.len(), 1);
        assert_eq!(snap.positions[0].lots, 10);
        assert!(snap.positions[0].unrealized_pnl_minor.is_some());
        assert!(snap.total_unrealized_pnl_minor.unwrap() > 0);
        assert!(snap.total_cost_basis_minor.is_some());
        assert_eq!(snap.realized_pnl_minor, 0);
        assert!(snap.total_equity_minor.is_some());
    }

    #[test]
    fn snapshot_after_partial_sell_shows_realized_pnl() {
        let mut engine = PaperEngine::new(phase1_master(), FaultConfig::happy_path());
        let acc = AccountId::from_u64(1);
        engine
            .deposit(
                acc,
                Money::from_major(10_000, Currency::usd()).expect("d"),
                "dep",
            )
            .expect("dep");
        let buy = SubmitRequest {
            account_id: acc,
            client_order_id: ClientOrderId::new("b1").expect("c"),
            instrument_id: aapl().id(),
            side: Side::Buy,
            qty: shinrai_instruments::QuantityLots::from_lots(10),
            price: PriceTicks::from_scaled(10_000),
        };
        engine.submit(&buy).expect("buy");
        let sell = SubmitRequest {
            account_id: acc,
            client_order_id: ClientOrderId::new("s1").expect("c"),
            instrument_id: aapl().id(),
            side: Side::Sell,
            qty: shinrai_instruments::QuantityLots::from_lots(4),
            price: PriceTicks::from_scaled(11_000),
        };
        engine.submit(&sell).expect("sell");

        let snap = build_snapshot(
            acc,
            engine.book(),
            engine.orders(),
            &phase1_master(),
            &HashMap::new(),
        )
        .expect("snap");
        assert_eq!(snap.positions[0].lots, 6);
        assert!(snap.realized_pnl_minor > 0);
    }
}
