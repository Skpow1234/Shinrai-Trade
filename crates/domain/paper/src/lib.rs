//! Paper trading loop: order → reserve → simulated venue → fill → settle.

#![forbid(unsafe_code)]

mod engine;
mod error;
mod notional;
mod reconcile;

pub use engine::{PaperEngine, SubmitRequest};
pub use error::PaperError;
pub use notional::notional;
pub use reconcile::{ReconciliationKind, ReconciliationMismatch, ReconciliationReport};
