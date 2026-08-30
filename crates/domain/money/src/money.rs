//! [`Money`] amounts in integer minor units.

use crate::currency::Currency;
use crate::error::MoneyError;
use core::fmt;

/// An exact monetary amount in a single [`Currency`].
///
/// The `minor_units` field is the integer number of smallest currency units
/// (for example cents when `currency.scale() == 2`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Money {
    minor_units: i128,
    currency: Currency,
}

impl Money {
    /// Creates money from an already-scaled minor-unit integer.
    #[must_use]
    pub const fn from_minor(minor_units: i128, currency: Currency) -> Self {
        Self {
            minor_units,
            currency,
        }
    }

    /// Creates money from a whole number of major units (no fractional part).
    ///
    /// # Errors
    ///
    /// Returns [`MoneyError::Overflow`] if `major * 10^scale` overflows `i128`.
    pub fn from_major(major: i128, currency: Currency) -> Result<Self, MoneyError> {
        let factor = currency.minor_factor()?;
        let minor = major.checked_mul(factor).ok_or(MoneyError::Overflow)?;
        Ok(Self::from_minor(minor, currency))
    }

    /// Parses a decimal major-unit string such as `"12.34"` or `"-7"`.
    ///
    /// The fractional part must not exceed the currency scale. Trailing zeros
    /// within the scale are allowed (`"1.5"` with scale 2 → 150 minor units).
    ///
    /// # Errors
    ///
    /// Returns a [`MoneyError`] if the string is malformed, overflows, or has
    /// too many fractional digits.
    pub fn parse_major(s: &str, currency: Currency) -> Result<Self, MoneyError> {
        let s = s.trim();
        if s.is_empty() {
            return Err(MoneyError::InvalidAmount);
        }

        let (sign, rest) = if let Some(rest) = s.strip_prefix('-') {
            (-1_i128, rest)
        } else if let Some(rest) = s.strip_prefix('+') {
            (1_i128, rest)
        } else {
            (1_i128, s)
        };

        if rest.is_empty() {
            return Err(MoneyError::InvalidAmount);
        }

        let (whole_str, frac_str) = match rest.split_once('.') {
            Some((w, f)) => (w, f),
            None => (rest, ""),
        };

        if whole_str.is_empty() || !whole_str.chars().all(|c| c.is_ascii_digit()) {
            return Err(MoneyError::InvalidAmount);
        }
        if !frac_str.chars().all(|c| c.is_ascii_digit()) {
            return Err(MoneyError::InvalidAmount);
        }
        if frac_str.len() > usize::from(currency.scale()) {
            return Err(MoneyError::ScaleExceeded {
                digits: frac_str.len(),
                scale: currency.scale(),
            });
        }

        let whole: i128 = whole_str.parse().map_err(|_| MoneyError::InvalidAmount)?;
        let factor = currency.minor_factor()?;
        let whole_minor = whole.checked_mul(factor).ok_or(MoneyError::Overflow)?;

        let mut frac_minor: i128 = if frac_str.is_empty() {
            0
        } else {
            frac_str.parse().map_err(|_| MoneyError::InvalidAmount)?
        };
        let pad = usize::from(currency.scale())
            .checked_sub(frac_str.len())
            .ok_or(MoneyError::Overflow)?;
        for _ in 0..pad {
            frac_minor = frac_minor.checked_mul(10).ok_or(MoneyError::Overflow)?;
        }

        let abs = whole_minor
            .checked_add(frac_minor)
            .ok_or(MoneyError::Overflow)?;
        let minor = abs.checked_mul(sign).ok_or(MoneyError::Overflow)?;
        Ok(Self::from_minor(minor, currency))
    }

    /// Returns the minor-unit integer.
    #[must_use]
    pub const fn minor_units(&self) -> i128 {
        self.minor_units
    }

    /// Returns the currency.
    #[must_use]
    pub const fn currency(&self) -> Currency {
        self.currency
    }

    /// Returns true when the amount is zero.
    #[must_use]
    pub const fn is_zero(&self) -> bool {
        self.minor_units == 0
    }

    /// Returns true when the amount is strictly negative.
    #[must_use]
    pub const fn is_negative(&self) -> bool {
        self.minor_units < 0
    }

    /// Checked negation.
    ///
    /// # Errors
    ///
    /// Returns [`MoneyError::Overflow`] if negation overflows (only for
    /// `i128::MIN`).
    pub fn checked_neg(self) -> Result<Self, MoneyError> {
        let minor = self.minor_units.checked_neg().ok_or(MoneyError::Overflow)?;
        Ok(Self::from_minor(minor, self.currency))
    }

    /// Absolute value with overflow check.
    ///
    /// # Errors
    ///
    /// Returns [`MoneyError::Overflow`] if the amount is `i128::MIN`.
    pub fn checked_abs(self) -> Result<Self, MoneyError> {
        if self.minor_units == i128::MIN {
            return Err(MoneyError::Overflow);
        }
        Ok(Self::from_minor(self.minor_units.abs(), self.currency))
    }

    fn ensure_same_currency(self, other: Self) -> Result<(), MoneyError> {
        if self.currency != other.currency {
            return Err(MoneyError::CurrencyMismatch {
                left: *self.currency.code().as_bytes(),
                right: *other.currency.code().as_bytes(),
            });
        }
        Ok(())
    }

    /// Checked addition; currencies must match.
    ///
    /// # Errors
    ///
    /// Returns [`MoneyError::CurrencyMismatch`] or [`MoneyError::Overflow`].
    pub fn checked_add(self, other: Self) -> Result<Self, MoneyError> {
        self.ensure_same_currency(other)?;
        let minor = self
            .minor_units
            .checked_add(other.minor_units)
            .ok_or(MoneyError::Overflow)?;
        Ok(Self::from_minor(minor, self.currency))
    }

    /// Checked subtraction; currencies must match.
    ///
    /// # Errors
    ///
    /// Returns [`MoneyError::CurrencyMismatch`] or [`MoneyError::Overflow`].
    pub fn checked_sub(self, other: Self) -> Result<Self, MoneyError> {
        self.ensure_same_currency(other)?;
        let minor = self
            .minor_units
            .checked_sub(other.minor_units)
            .ok_or(MoneyError::Overflow)?;
        Ok(Self::from_minor(minor, self.currency))
    }

    /// Multiplies by a scalar integer with overflow checking.
    ///
    /// # Errors
    ///
    /// Returns [`MoneyError::Overflow`] on overflow.
    pub fn checked_mul_scalar(self, scalar: i128) -> Result<Self, MoneyError> {
        let minor = self
            .minor_units
            .checked_mul(scalar)
            .ok_or(MoneyError::Overflow)?;
        Ok(Self::from_minor(minor, self.currency))
    }
}

impl fmt::Display for Money {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let scale = self.currency.scale();
        let Ok(factor) = self.currency.minor_factor() else {
            return Err(fmt::Error);
        };

        let negative = self.minor_units < 0;
        let abs = self.minor_units.unsigned_abs();
        let factor_u = factor.unsigned_abs();
        let whole = abs / factor_u;
        let frac = abs % factor_u;

        if negative {
            write!(f, "-")?;
        }
        write!(f, "{whole}")?;
        if scale > 0 {
            write!(f, ".{frac:0width$}", width = usize::from(scale))?;
        }
        write!(f, " {}", self.currency.code())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_display_usd() {
        let m = Money::parse_major("12.34", Currency::usd()).expect("parse");
        assert_eq!(m.minor_units(), 1234);
        assert_eq!(m.to_string(), "12.34 USD");
    }

    #[test]
    fn parse_partial_fraction_pads_scale() {
        let m = Money::parse_major("1.5", Currency::usd()).expect("parse");
        assert_eq!(m.minor_units(), 150);
        assert_eq!(m.to_string(), "1.50 USD");
    }

    #[test]
    fn parse_rejects_excess_fraction() {
        let err = Money::parse_major("1.234", Currency::usd()).expect_err("too many digits");
        assert!(matches!(
            err,
            MoneyError::ScaleExceeded {
                digits: 3,
                scale: 2
            }
        ));
    }

    #[test]
    fn jpy_has_no_fraction_in_display() {
        let m = Money::from_major(1500, Currency::jpy()).expect("major");
        assert_eq!(m.to_string(), "1500 JPY");
    }

    #[test]
    fn add_sub_same_currency() {
        let a = Money::parse_major("10.00", Currency::usd()).expect("a");
        let b = Money::parse_major("2.50", Currency::usd()).expect("b");
        let sum = a.checked_add(b).expect("add");
        assert_eq!(sum.minor_units(), 1250);
        let diff = sum.checked_sub(b).expect("sub");
        assert_eq!(diff, a);
    }

    #[test]
    fn rejects_currency_mismatch() {
        let a = Money::from_major(1, Currency::usd()).expect("usd");
        let b = Money::from_major(1, Currency::eur()).expect("eur");
        assert!(matches!(
            a.checked_add(b),
            Err(MoneyError::CurrencyMismatch { .. })
        ));
    }

    #[test]
    fn mul_scalar_and_neg() {
        let a = Money::parse_major("3.00", Currency::usd()).expect("a");
        let b = a.checked_mul_scalar(4).expect("mul");
        assert_eq!(b.minor_units(), 1200);
        let n = b.checked_neg().expect("neg");
        assert_eq!(n.minor_units(), -1200);
    }

    #[test]
    fn from_major_overflow() {
        let err = Money::from_major(i128::MAX, Currency::usd()).expect_err("overflow");
        assert_eq!(err, MoneyError::Overflow);
    }

    #[test]
    fn zero_helpers() {
        let z = Money::from_minor(0, Currency::usd());
        assert!(z.is_zero());
        assert!(!z.is_negative());
    }
}
