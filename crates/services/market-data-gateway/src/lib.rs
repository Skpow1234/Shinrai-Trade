//! HTTP/WebSocket market-data gateway.
//!
//! Domain fanout stays in [`shinrai_md_fanout`]. This crate owns sockets,
//! HTTP, and mapping [`shinrai_md_protocol::SupervisorEvent`] onto client
//! frames. Tokens are never logged.

#![forbid(unsafe_code)]

mod app;
mod auth_http;
mod coinbase_feed;
mod historical;
mod map;

pub use app::{router, unix_logical_now, AppState, FeedMode, GatewayConfig};
pub use map::to_market_event;
