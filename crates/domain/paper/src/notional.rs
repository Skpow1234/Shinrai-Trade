//! Integer notional: price ticks × lots × multiplier, rescaled to quote cash.

use shinrai_instruments::{Instrument, PriceTicks, QuantityLots};
use shinrai_money::{Money, MoneyError};
use shinrai_orders::OrderError;

use crate::error::PaperError;

/// Computes quote-currency notional for a limit/fill.
///
/// `notional_minor = price_scaled * qty_lots * multiplier`, then converted
/// from the instrument `price_scale` to the quote currency scale. Conversion
/// must be exact (no rounding).
///
/// # Errors
///
/// Returns overflow, inexact scale conversion, or invalid quantity/price.
pub fn notional(
    instrument: &Instrument,
    price: PriceTicks,
    qty: QuantityLots,
) -> Result<Money, PaperError> {
    if qty.lots() <= 0 {
        return Err(PaperError::Order(OrderError::InvalidQuantity));
    }
    if price.scaled() <= 0 {
        return Err(PaperError::Order(OrderError::InvalidPrice));
    }

    let raw = i128::from(price.scaled())
        .checked_mul(i128::from(qty.lots()))
        .and_then(|v| v.checked_mul(i128::from(instrument.multiplier())))
        .ok_or(MoneyError::Overflow)?;

    let from = i32::from(instrument.tick_table().price_scale());
    let to = i32::from(instrument.quote_currency().scale());
    let minor = rescale_exact(raw, from, to)?;
    Ok(Money::from_minor(minor, instrument.quote_currency()))
}

fn rescale_exact(amount: i128, from_scale: i32, to_scale: i32) -> Result<i128, PaperError> {
    match to_scale.cmp(&from_scale) {
        std::cmp::Ordering::Equal => Ok(amount),
        std::cmp::Ordering::Greater => {
            let exp =
                u32::try_from(to_scale - from_scale).map_err(|_| PaperError::InexactNotional)?;
            let factor = 10_i128.checked_pow(exp).ok_or(MoneyError::Overflow)?;
            amount
                .checked_mul(factor)
                .ok_or(PaperError::Money(MoneyError::Overflow))
        }
        std::cmp::Ordering::Less => {
            let exp =
                u32::try_from(from_scale - to_scale).map_err(|_| PaperError::InexactNotional)?;
            let factor = 10_i128.checked_pow(exp).ok_or(MoneyError::Overflow)?;
            if amount.rem_euclid(factor) != 0 {
                return Err(PaperError::InexactNotional);
            }
            Ok(amount / factor)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shinrai_instruments::aapl;

    #[test]
    fn aapl_notional_matches_cash() {
        let inst = aapl();
        // 100.00 * 10 shares = 1000.00 USD = 100_000 cents
        let n = notional(
            &inst,
            PriceTicks::from_scaled(10_000),
            QuantityLots::from_lots(10),
        )
        .expect("n");
        assert_eq!(n.minor_units(), 100_000);
    }
}
