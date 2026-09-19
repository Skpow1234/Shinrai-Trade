//! Sink trait and log implementation.

use std::fmt::Debug;
use std::future::Future;
use std::pin::Pin;

use serde_json::Value;
use thiserror::Error;

/// Wire envelope published to the bus (and logged).
#[derive(Debug, Clone)]
pub struct EventEnvelope {
    /// Durable outbox row id.
    pub event_id: i64,
    /// Logical topic (`ledger.posted`, `order.upserted`, …).
    pub topic: String,
    /// JSON payload (must not contain secrets).
    pub payload: Value,
}

/// Failures delivering to an external bus.
#[derive(Debug, Error)]
pub enum MessagingError {
    /// NATS client / publish failure.
    #[error("nats: {0}")]
    Nats(String),
    /// Payload could not be serialized.
    #[error("serialize: {0}")]
    Serialize(#[from] serde_json::Error),
}

/// Which sink the process is using (ops / startup logs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SinkKind {
    /// `tracing` only.
    Log,
    /// NATS core publish.
    Nats,
}

/// Async deliver target for outbox rows.
pub trait EventSink: Send + Sync + Debug {
    /// Human-readable kind for startup / metrics.
    fn kind(&self) -> SinkKind;

    /// Deliver one event. On error the outbox row must stay unpublished.
    fn publish<'a>(
        &'a self,
        event: &'a EventEnvelope,
    ) -> Pin<Box<dyn Future<Output = Result<(), MessagingError>> + Send + 'a>>;
}

/// Stand-in bus: emit `shinrai_outbox` / `outbox.delivered` via tracing.
#[derive(Debug, Default, Clone, Copy)]
pub struct LogSink;

impl EventSink for LogSink {
    fn kind(&self) -> SinkKind {
        SinkKind::Log
    }

    fn publish<'a>(
        &'a self,
        event: &'a EventEnvelope,
    ) -> Pin<Box<dyn Future<Output = Result<(), MessagingError>> + Send + 'a>> {
        Box::pin(async move {
            tracing::info!(
                target: "shinrai_outbox",
                event_id = event.event_id,
                topic = %event.topic,
                payload = %event.payload,
                "outbox.delivered"
            );
            Ok(())
        })
    }
}
