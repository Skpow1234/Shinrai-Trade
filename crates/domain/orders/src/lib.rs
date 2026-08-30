//! Order lifecycle state machine and idempotent order store.
//!
//! Orders are an explicit finite-state machine. Status is never a bag of
//! booleans. Transitions are driven by [`OrderEvent`] values only.

#![forbid(unsafe_code)]

mod error;
mod event;
mod ids;
mod order;
mod status;
mod store;
mod transition;

pub use error::OrderError;
pub use event::{DomainEffect, OrderEvent};
pub use ids::{ClientOrderId, ExecId, OrderId, VenueOrderId};
pub use order::{Order, OrderType, Side};
pub use status::OrderStatus;
pub use store::{CreateOrder, LoggedEvent, OrderStore, SubmitOutcome};
pub use transition::apply;
