//! Instrument master, trading grids, and symbol alias resolution.
//!
//! Prices and quantities are integer **ticks** / **lots** inside the domain.
//! Human decimal strings are converted only at the boundary.

#![forbid(unsafe_code)]

mod error;
mod fixtures;
mod grid;
mod ids;
mod instrument;
mod isin;
mod master;
mod types;

pub use error::InstrumentError;
pub use fixtures::{aapl, btc_usd, esz5, phase1_master};
pub use grid::{LotSpec, PriceTicks, QuantityLots, TickBand, TickTable};
pub use ids::{ExternalId, IdType, InstrumentId};
pub use instrument::Instrument;
pub use isin::validate_isin;
pub use master::InstrumentMaster;
pub use types::{AssetClass, InstrumentStatus, InstrumentType};
