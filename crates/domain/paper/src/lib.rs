//! Paper trading loop: order → reserve → simulated venue → fill → settle.

#![forbid(unsafe_code)]

mod engine;
mod error;
mod notional;
mod reconcile;
mod risk_ctx;
mod venue;

pub use engine::{PaperEngine, ReplaceRequest, SubmitRequest};
pub use error::PaperError;
pub use notional::notional;
pub use reconcile::{
    BrokerEodCash, BrokerEodFill, BrokerEodPosition, BrokerEodSnapshot, ReconciliationKind,
    ReconciliationMismatch, ReconciliationReport,
};
pub use risk_ctx::{utc_day_id, utc_day_start, SECS_PER_DAY};
pub use venue::VenueKind;
