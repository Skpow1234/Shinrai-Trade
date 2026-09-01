//! HTTP order gateway.
//!
//! Authenticated paper order submit/cancel over Axum. Domain logic stays in
//! [`shinrai_paper`] and [`shinrai_risk`]; tokens are never logged.

#![forbid(unsafe_code)]

mod app;
mod auth_http;
mod md_client;
mod orders_http;
mod portfolio_http;

pub use app::{router, unix_logical_now, AppState, GatewayConfig};
