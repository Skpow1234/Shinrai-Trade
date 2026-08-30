//! Tick ladders, lot specs, and decimal ↔ integer conversions.

use core::fmt;

use crate::error::InstrumentError;

/// Maximum fractional digits for price / quantity scales.
pub const MAX_SCALE: u8 = 18;

/// On-grid price in the instrument's scaled price units.
///
/// For a constant tick size, the tick count is `scaled / tick_size_scaled`.
/// With variable tick ladders, this value remains the scaled price that has
/// already been validated against the applicable band.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PriceTicks(i64);

impl PriceTicks {
    /// Wraps an already-validated scaled price.
    #[must_use]
    pub const fn from_scaled(scaled: i64) -> Self {
        Self(scaled)
    }

    /// Returns the scaled integer price.
    #[must_use]
    pub const fn scaled(self) -> i64 {
        self.0
    }
}

impl fmt::Display for PriceTicks {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// On-grid quantity expressed as a multiple of the instrument lot step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct QuantityLots(i64);

impl QuantityLots {
    /// Wraps an already-validated lot count.
    #[must_use]
    pub const fn from_lots(lots: i64) -> Self {
        Self(lots)
    }

    /// Returns the lot count.
    #[must_use]
    pub const fn lots(self) -> i64 {
        self.0
    }
}

impl fmt::Display for QuantityLots {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// One band of a (possibly variable) tick table.
///
/// Bounds are in scaled price units for the instrument's `price_scale`.
/// The half-open interval is `[min, max)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TickBand {
    /// Inclusive lower bound in scaled price units.
    min: i64,
    /// Exclusive upper bound in scaled price units; `None` = unbounded.
    max: Option<i64>,
    /// Tick increment in scaled price units (must be > 0).
    tick_size: i64,
}

impl TickBand {
    /// Creates a tick band.
    ///
    /// # Errors
    ///
    /// Returns [`InstrumentError::InvalidGrid`] if the tick size is not positive
    /// or the bounds are inconsistent.
    pub const fn new(min: i64, max: Option<i64>, tick_size: i64) -> Result<Self, InstrumentError> {
        if tick_size <= 0 {
            return Err(InstrumentError::InvalidGrid("tick size must be positive"));
        }
        if let Some(max_bound) = max {
            if max_bound <= min {
                return Err(InstrumentError::InvalidGrid(
                    "tick band max must be greater than min",
                ));
            }
        }
        Ok(Self {
            min,
            max,
            tick_size,
        })
    }

    /// Inclusive lower bound (scaled).
    #[must_use]
    pub const fn min_scaled(self) -> i64 {
        self.min
    }

    /// Exclusive upper bound (scaled), if any.
    #[must_use]
    pub const fn max_scaled(self) -> Option<i64> {
        self.max
    }

    /// Tick increment (scaled).
    #[must_use]
    pub const fn tick_size_scaled(self) -> i64 {
        self.tick_size
    }

    const fn contains(self, scaled: i64) -> bool {
        if scaled < self.min {
            return false;
        }
        match self.max {
            Some(max) => scaled < max,
            None => true,
        }
    }
}

/// Ordered tick ladder for an instrument.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TickTable {
    bands: Vec<TickBand>,
    price_scale: u8,
}

impl TickTable {
    /// Creates a tick table from bands (must be non-empty, contiguous, sorted).
    ///
    /// # Errors
    ///
    /// Returns [`InstrumentError`] if the scale or band layout is invalid.
    pub fn new(price_scale: u8, bands: Vec<TickBand>) -> Result<Self, InstrumentError> {
        if price_scale > MAX_SCALE {
            return Err(InstrumentError::ScaleOutOfRange { scale: price_scale });
        }
        if bands.is_empty() {
            return Err(InstrumentError::InvalidGrid(
                "tick table needs at least one band",
            ));
        }
        for window in bands.windows(2) {
            let left = window[0];
            let right = window[1];
            let Some(left_max) = left.max else {
                return Err(InstrumentError::InvalidGrid(
                    "only the last tick band may be unbounded",
                ));
            };
            if left_max != right.min {
                return Err(InstrumentError::InvalidGrid(
                    "tick bands must be contiguous",
                ));
            }
            if left.min > right.min {
                return Err(InstrumentError::InvalidGrid("tick bands must be sorted"));
            }
        }
        Ok(Self { bands, price_scale })
    }

    /// Single constant tick size covering all prices from `0` upward.
    ///
    /// # Errors
    ///
    /// Returns an error if `tick_size_scaled` is not positive or scale is invalid.
    pub fn constant(price_scale: u8, tick_size_scaled: i64) -> Result<Self, InstrumentError> {
        // Allow negative prices via a symmetric band from i64::MIN/2 style is complex;
        // Phase 1 constant tables start at 0 for equities/crypto mids. Use full-range
        // band for signed prices.
        let band = TickBand::new(i64::MIN, None, tick_size_scaled)?;
        Self::new(price_scale, vec![band])
    }

    /// Returns the price scale.
    #[must_use]
    pub const fn price_scale(&self) -> u8 {
        self.price_scale
    }

    /// Returns the bands.
    #[must_use]
    pub fn bands(&self) -> &[TickBand] {
        &self.bands
    }

    /// Finds the band containing `scaled`.
    ///
    /// # Errors
    ///
    /// Returns [`InstrumentError::PriceOutOfBands`] if no band matches.
    pub fn band_for(&self, scaled: i64) -> Result<TickBand, InstrumentError> {
        self.bands
            .iter()
            .copied()
            .find(|b| b.contains(scaled))
            .ok_or(InstrumentError::PriceOutOfBands { scaled })
    }

    /// Validates that `scaled` sits on the tick grid.
    ///
    /// # Errors
    ///
    /// Returns off-grid or out-of-band errors.
    pub fn validate_scaled(&self, scaled: i64) -> Result<PriceTicks, InstrumentError> {
        let band = self.band_for(scaled)?;
        let tick = band.tick_size;
        if scaled.rem_euclid(tick) != 0 {
            return Err(InstrumentError::PriceOffGrid {
                scaled,
                tick_size_scaled: tick,
            });
        }
        Ok(PriceTicks::from_scaled(scaled))
    }

    /// Converts a major-unit decimal price string into on-grid [`PriceTicks`].
    ///
    /// # Errors
    ///
    /// Returns parse / scale / grid errors.
    pub fn price_to_ticks(&self, decimal: &str) -> Result<PriceTicks, InstrumentError> {
        let scaled = parse_scaled(decimal, self.price_scale)?;
        self.validate_scaled(scaled)
    }

    /// Formats on-grid ticks back to a major-unit decimal string (no currency).
    ///
    /// # Errors
    ///
    /// Returns [`InstrumentError::Overflow`] if formatting factors overflow.
    pub fn ticks_to_price(&self, ticks: PriceTicks) -> Result<String, InstrumentError> {
        format_scaled(ticks.scaled(), self.price_scale)
    }
}

/// Lot / step constraints for order quantity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LotSpec {
    /// Size of one lot in scaled quantity units (must be > 0).
    lot_size_scaled: i64,
    /// Minimum order quantity in scaled units.
    min_qty_scaled: i64,
    /// Optional maximum order quantity in scaled units.
    max_qty_scaled: Option<i64>,
    /// Order quantity must be a multiple of this step (usually == lot size).
    step_scaled: i64,
    /// Quantity fractional scale (decimal places).
    qty_scale: u8,
}

impl LotSpec {
    /// Creates a lot specification.
    ///
    /// # Errors
    ///
    /// Returns [`InstrumentError::InvalidGrid`] or scale errors.
    pub fn new(
        qty_scale: u8,
        lot_size_scaled: i64,
        min_qty_scaled: i64,
        max_qty_scaled: Option<i64>,
        step_scaled: i64,
    ) -> Result<Self, InstrumentError> {
        if qty_scale > MAX_SCALE {
            return Err(InstrumentError::ScaleOutOfRange { scale: qty_scale });
        }
        if lot_size_scaled <= 0 || step_scaled <= 0 {
            return Err(InstrumentError::InvalidGrid(
                "lot size and step must be positive",
            ));
        }
        if min_qty_scaled <= 0 {
            return Err(InstrumentError::InvalidGrid(
                "min quantity must be positive",
            ));
        }
        if min_qty_scaled.rem_euclid(step_scaled) != 0 {
            return Err(InstrumentError::InvalidGrid(
                "min quantity must be a multiple of step",
            ));
        }
        if let Some(max) = max_qty_scaled {
            if max < min_qty_scaled {
                return Err(InstrumentError::InvalidGrid(
                    "max quantity must be >= min quantity",
                ));
            }
            if max.rem_euclid(step_scaled) != 0 {
                return Err(InstrumentError::InvalidGrid(
                    "max quantity must be a multiple of step",
                ));
            }
        }
        Ok(Self {
            lot_size_scaled,
            min_qty_scaled,
            max_qty_scaled,
            step_scaled,
            qty_scale,
        })
    }

    /// Whole-share equity lot of 1 with no max.
    ///
    /// # Errors
    ///
    /// Returns an error if construction fails (should not for these constants).
    pub fn whole_shares() -> Result<Self, InstrumentError> {
        Self::new(0, 1, 1, None, 1)
    }

    /// Returns the quantity scale.
    #[must_use]
    pub const fn qty_scale(self) -> u8 {
        self.qty_scale
    }

    /// Returns the lot size in scaled units.
    #[must_use]
    pub const fn lot_size_scaled(self) -> i64 {
        self.lot_size_scaled
    }

    /// Returns the step in scaled units.
    #[must_use]
    pub const fn step_scaled(self) -> i64 {
        self.step_scaled
    }

    /// Validates a scaled quantity and returns lot count (`scaled / step`).
    ///
    /// # Errors
    ///
    /// Returns min/max/step alignment errors.
    pub fn validate_scaled(&self, scaled: i64) -> Result<QuantityLots, InstrumentError> {
        if scaled < self.min_qty_scaled {
            return Err(InstrumentError::QuantityBelowMin {
                scaled,
                min_scaled: self.min_qty_scaled,
            });
        }
        if let Some(max) = self.max_qty_scaled {
            if scaled > max {
                return Err(InstrumentError::QuantityAboveMax {
                    scaled,
                    max_scaled: max,
                });
            }
        }
        if scaled.rem_euclid(self.step_scaled) != 0 {
            return Err(InstrumentError::QuantityOffGrid {
                scaled,
                step_scaled: self.step_scaled,
            });
        }
        Ok(QuantityLots::from_lots(scaled / self.step_scaled))
    }

    /// Converts a market-data size into lots. Zero is allowed (delete a level);
    /// order min/max are not applied.
    ///
    /// # Errors
    ///
    /// Returns parse / step-alignment errors. Negative sizes are rejected.
    pub fn size_to_lots(&self, decimal: &str) -> Result<QuantityLots, InstrumentError> {
        let scaled = parse_scaled(decimal, self.qty_scale)?;
        if scaled == 0 {
            return Ok(QuantityLots::from_lots(0));
        }
        if scaled < 0 {
            return Err(InstrumentError::QuantityBelowMin {
                scaled,
                min_scaled: 0,
            });
        }
        if scaled.rem_euclid(self.step_scaled) != 0 {
            return Err(InstrumentError::QuantityOffGrid {
                scaled,
                step_scaled: self.step_scaled,
            });
        }
        Ok(QuantityLots::from_lots(scaled / self.step_scaled))
    }

    /// Converts a major-unit decimal quantity into [`QuantityLots`].
    ///
    /// # Errors
    ///
    /// Returns parse / grid errors.
    pub fn qty_to_lots(&self, decimal: &str) -> Result<QuantityLots, InstrumentError> {
        let scaled = parse_scaled(decimal, self.qty_scale)?;
        self.validate_scaled(scaled)
    }

    /// Formats lot count back to a major-unit decimal string.
    ///
    /// # Errors
    ///
    /// Returns overflow errors.
    pub fn lots_to_qty(&self, lots: QuantityLots) -> Result<String, InstrumentError> {
        let scaled = lots
            .lots()
            .checked_mul(self.step_scaled)
            .ok_or(InstrumentError::Overflow)?;
        format_scaled(scaled, self.qty_scale)
    }
}

/// Parses a decimal major-unit string into a scaled integer.
pub(crate) fn parse_scaled(s: &str, scale: u8) -> Result<i64, InstrumentError> {
    let s = s.trim();
    if s.is_empty() {
        return Err(InstrumentError::InvalidDecimal);
    }

    let (sign, rest) = if let Some(rest) = s.strip_prefix('-') {
        (-1_i64, rest)
    } else if let Some(rest) = s.strip_prefix('+') {
        (1_i64, rest)
    } else {
        (1_i64, s)
    };

    if rest.is_empty() {
        return Err(InstrumentError::InvalidDecimal);
    }

    let (whole_str, frac_str) = match rest.split_once('.') {
        Some((w, f)) => (w, f),
        None => (rest, ""),
    };

    if whole_str.is_empty() || !whole_str.chars().all(|c| c.is_ascii_digit()) {
        return Err(InstrumentError::InvalidDecimal);
    }
    if !frac_str.chars().all(|c| c.is_ascii_digit()) {
        return Err(InstrumentError::InvalidDecimal);
    }
    if frac_str.len() > usize::from(scale) {
        return Err(InstrumentError::ScaleExceeded {
            digits: frac_str.len(),
            scale,
        });
    }

    let whole: i64 = whole_str
        .parse()
        .map_err(|_| InstrumentError::InvalidDecimal)?;
    let factor = pow10_i64(scale)?;
    let whole_scaled = whole.checked_mul(factor).ok_or(InstrumentError::Overflow)?;

    let mut frac: i64 = if frac_str.is_empty() {
        0
    } else {
        frac_str
            .parse()
            .map_err(|_| InstrumentError::InvalidDecimal)?
    };
    let pad = usize::from(scale)
        .checked_sub(frac_str.len())
        .ok_or(InstrumentError::Overflow)?;
    for _ in 0..pad {
        frac = frac.checked_mul(10).ok_or(InstrumentError::Overflow)?;
    }

    let abs = whole_scaled
        .checked_add(frac)
        .ok_or(InstrumentError::Overflow)?;
    abs.checked_mul(sign).ok_or(InstrumentError::Overflow)
}

fn format_scaled(scaled: i64, scale: u8) -> Result<String, InstrumentError> {
    let factor = pow10_i64(scale)?;
    let negative = scaled < 0;
    let abs = scaled.unsigned_abs();
    let factor_u = factor.unsigned_abs();
    let whole = abs / factor_u;
    let frac = abs % factor_u;

    let mut out = String::new();
    if negative {
        out.push('-');
    }
    out.push_str(&whole.to_string());
    if scale > 0 {
        out.push('.');
        let frac_str = format!("{frac:0width$}", width = usize::from(scale));
        out.push_str(&frac_str);
    }
    Ok(out)
}

fn pow10_i64(scale: u8) -> Result<i64, InstrumentError> {
    10_i64
        .checked_pow(u32::from(scale))
        .ok_or(InstrumentError::Overflow)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_tick_round_trip() {
        let table = TickTable::constant(2, 1).expect("table"); // tick = 0.01
        let ticks = table.price_to_ticks("12.34").expect("on grid");
        assert_eq!(ticks.scaled(), 1234);
        assert_eq!(table.ticks_to_price(ticks).expect("fmt"), "12.34");
    }

    #[test]
    fn rejects_off_grid_price() {
        let table = TickTable::constant(2, 5).expect("table"); // tick = 0.05
        let err = table.price_to_ticks("12.34").expect_err("off grid");
        assert!(matches!(err, InstrumentError::PriceOffGrid { .. }));
    }

    #[test]
    fn variable_tick_ladder() {
        let bands = vec![
            TickBand::new(0, Some(1000), 1).expect("b0"), // [0, 10.00) step 0.01
            TickBand::new(1000, None, 5).expect("b1"),    // [10.00, ∞) step 0.05
        ];
        let table = TickTable::new(2, bands).expect("table");
        assert_eq!(table.price_to_ticks("9.99").expect("ok").scaled(), 999);
        assert!(table.price_to_ticks("10.05").is_ok());
        assert!(matches!(
            table.price_to_ticks("10.01"),
            Err(InstrumentError::PriceOffGrid { .. })
        ));
    }

    #[test]
    fn lot_round_trip() {
        let lots = LotSpec::new(8, 1, 1, None, 1).expect("lots"); // 1e-8 step
        let q = lots.qty_to_lots("0.00000001").expect("ok");
        assert_eq!(q.lots(), 1);
        assert_eq!(lots.lots_to_qty(q).expect("fmt"), "0.00000001");
    }

    #[test]
    fn size_zero_deletes_book_level() {
        let lots = LotSpec::new(8, 1, 1, None, 1).expect("lots");
        assert_eq!(lots.size_to_lots("0").expect("z").lots(), 0);
        assert_eq!(lots.size_to_lots("0.00000000").expect("z8").lots(), 0);
        assert!(lots.qty_to_lots("0").is_err());
    }
}
