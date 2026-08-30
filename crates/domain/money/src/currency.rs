//! Currency codes and decimal scales.

use crate::error::MoneyError;
use core::fmt;
use core::str::FromStr;

/// Maximum supported fractional digits for a currency scale.
pub const MAX_SCALE: u8 = 18;

/// ISO-like 3-character currency code (ASCII uppercase).
///
/// Stored as three bytes. Non-ISO / crypto codes are allowed as long as they
/// are exactly three `A–Z` characters (for example `BTC` as a placeholder until
/// a dedicated crypto asset type exists).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CurrencyCode([u8; 3]);

impl CurrencyCode {
    /// Creates a currency code from three ASCII alphabetic characters.
    ///
    /// # Errors
    ///
    /// Returns [`MoneyError::InvalidCurrencyCode`] if the input is not exactly
    /// three ASCII letters.
    pub fn new(code: &str) -> Result<Self, MoneyError> {
        let bytes = code.as_bytes();
        if bytes.len() != 3 {
            return Err(MoneyError::InvalidCurrencyCode);
        }
        let mut out = [0_u8; 3];
        for (i, &b) in bytes.iter().enumerate() {
            if !b.is_ascii_alphabetic() {
                return Err(MoneyError::InvalidCurrencyCode);
            }
            out[i] = b.to_ascii_uppercase();
        }
        Ok(Self(out))
    }

    /// Returns the code as a string slice (always valid UTF-8).
    #[must_use]
    pub fn as_str(&self) -> &str {
        // Bytes are always ASCII A–Z by construction.
        match core::str::from_utf8(&self.0) {
            Ok(s) => s,
            Err(_) => "???",
        }
    }

    /// Infallible constructor for validated uppercase ASCII codes.
    const fn from_bytes_unchecked(bytes: [u8; 3]) -> Self {
        Self(bytes)
    }

    /// Returns the raw three-byte representation.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 3] {
        &self.0
    }
}

impl fmt::Display for CurrencyCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for CurrencyCode {
    type Err = MoneyError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

/// A currency with a fixed minor-unit scale.
///
/// `scale` is the number of decimal digits after the major-unit radix point.
/// USD uses `2` (cents); JPY uses `0`; many crypto quote conventions use `8`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Currency {
    code: CurrencyCode,
    scale: u8,
}

impl Currency {
    /// Creates a currency from a code string and scale.
    ///
    /// # Errors
    ///
    /// Returns an error if the code is invalid or `scale` exceeds [`MAX_SCALE`].
    pub fn new(code: &str, scale: u8) -> Result<Self, MoneyError> {
        if scale > MAX_SCALE {
            return Err(MoneyError::ScaleOutOfRange { scale });
        }
        Ok(Self {
            code: CurrencyCode::new(code)?,
            scale,
        })
    }

    /// Creates a currency from an existing code and scale.
    ///
    /// # Errors
    ///
    /// Returns [`MoneyError::ScaleOutOfRange`] if `scale` exceeds [`MAX_SCALE`].
    pub const fn from_code(code: CurrencyCode, scale: u8) -> Result<Self, MoneyError> {
        if scale > MAX_SCALE {
            return Err(MoneyError::ScaleOutOfRange { scale });
        }
        Ok(Self { code, scale })
    }

    /// USD — United States dollar (2 decimal places).
    #[must_use]
    pub const fn usd() -> Self {
        Self {
            code: CurrencyCode::from_bytes_unchecked(*b"USD"),
            scale: 2,
        }
    }

    /// EUR — Euro (2 decimal places).
    #[must_use]
    pub const fn eur() -> Self {
        Self {
            code: CurrencyCode::from_bytes_unchecked(*b"EUR"),
            scale: 2,
        }
    }

    /// JPY — Japanese yen (0 decimal places).
    #[must_use]
    pub const fn jpy() -> Self {
        Self {
            code: CurrencyCode::from_bytes_unchecked(*b"JPY"),
            scale: 0,
        }
    }

    /// GBP — Pound sterling (2 decimal places).
    #[must_use]
    pub const fn gbp() -> Self {
        Self {
            code: CurrencyCode::from_bytes_unchecked(*b"GBP"),
            scale: 2,
        }
    }

    /// Returns the currency code.
    #[must_use]
    pub const fn code(&self) -> CurrencyCode {
        self.code
    }

    /// Returns the number of fractional digits (minor-unit scale).
    #[must_use]
    pub const fn scale(&self) -> u8 {
        self.scale
    }

    /// Factor that converts one major unit into minor units (`10^scale`).
    ///
    /// # Errors
    ///
    /// Returns [`MoneyError::Overflow`] if the factor does not fit in `i128`
    /// (should not happen for `scale <= MAX_SCALE`).
    pub fn minor_factor(&self) -> Result<i128, MoneyError> {
        10_i128
            .checked_pow(u32::from(self.scale))
            .ok_or(MoneyError::Overflow)
    }
}

impl fmt::Display for Currency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}(scale={})", self.code, self.scale)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn currency_code_normalizes_case() {
        let code = CurrencyCode::new("usd").expect("valid");
        assert_eq!(code.as_str(), "USD");
    }

    #[test]
    fn currency_code_rejects_bad_length() {
        assert!(CurrencyCode::new("US").is_err());
        assert!(CurrencyCode::new("USDT").is_err());
    }

    #[test]
    fn currency_code_rejects_non_alpha() {
        assert!(CurrencyCode::new("US1").is_err());
    }

    #[test]
    fn scale_out_of_range() {
        assert!(matches!(
            Currency::new("USD", 19),
            Err(MoneyError::ScaleOutOfRange { scale: 19 })
        ));
    }

    #[test]
    fn minor_factor_matches_scale() {
        assert_eq!(Currency::usd().minor_factor().expect("ok"), 100);
        assert_eq!(Currency::jpy().minor_factor().expect("ok"), 1);
        assert_eq!(
            Currency::new("BTC", 8)
                .expect("ok")
                .minor_factor()
                .expect("ok"),
            100_000_000
        );
    }
}
