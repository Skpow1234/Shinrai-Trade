//! HTTP order gateway.
//!
//! Authenticated paper order submit/cancel over Axum. Domain logic stays in
//! [`shinrai_paper`] and [`shinrai_risk`]; tokens are never logged.

#![forbid(unsafe_code)]

mod app;
mod auth_http;
mod hydrate;
mod md_client;
mod ops;
mod ops_allowlist;
mod ops_http;
mod orders_http;
mod outbox_publisher;
mod persist;
mod portfolio_http;
mod rate_limit;

pub use app::{router, unix_logical_now, AppState, GatewayConfig};
pub use outbox_publisher::{
    publish_once as publish_outbox_once, run_publisher as run_outbox_publisher,
};
pub use rate_limit::RateLimiter;
