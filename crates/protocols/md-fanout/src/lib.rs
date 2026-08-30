//! Client market-data fanout.
//!
//! Transport-agnostic: a WebSocket gateway owns I/O and drives [`FanoutHub`].
//! Market-data queues are **bounded**; overflow drops the oldest outbound
//! message (never blocks). Orders are not fanned out on this path.

#![forbid(unsafe_code)]

mod auth;
mod error;
mod hub;
mod protocol;
mod session;

pub use auth::{Authenticator, SessionClaims, StaticTokenAuth, SubjectId};
pub use error::FanoutError;
pub use hub::{ClockOutcome, CloseReason, FanoutConfig, FanoutHub};
pub use protocol::{decode_command, encode_message, ClientCommand};
pub use session::{ClientMessage, MarketEvent, SessionId};
