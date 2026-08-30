//! Exact monetary amounts and currency scales.
//!
//! Floating-point types (`f32` / `f64`) are forbidden on the money path.
//! Amounts are stored as integer **minor units** (for example cents for USD)
//! paired with a [`Currency`] that defines the scale.

#![forbid(unsafe_code)]

mod currency;
mod error;
mod money;

pub use currency::{Currency, CurrencyCode};
pub use error::MoneyError;
pub use money::Money;
