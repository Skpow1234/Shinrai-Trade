//! Background outbox publisher: claim → sink → mark published.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use shinrai_messaging::{EventEnvelope, EventSink, LogSink, MessagingError};
use shinrai_store::{claim_unpublished, mark_published, OutboxEvent, StoreError, StorePool};
use thiserror::Error;

/// Shared counters for ops metrics.
#[derive(Debug, Default)]
pub struct OutboxMetrics {
    published: AtomicU64,
    publish_errors: AtomicU64,
    /// Retained for ops JSON compatibility (inbox no longer gates publisher).
    duplicates_skipped: AtomicU64,
}

impl OutboxMetrics {
    /// JSON-friendly snapshot.
    #[must_use]
    pub fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "outbox_published": self.published.load(Ordering::Relaxed),
            "outbox_publish_errors": self.publish_errors.load(Ordering::Relaxed),
            "outbox_duplicates_skipped": self.duplicates_skipped.load(Ordering::Relaxed),
        })
    }
}

/// Outbox poll / deliver failures.
#[derive(Debug, Error)]
pub enum OutboxPublishError {
    /// Postgres claim / mark failure.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// External bus (NATS) failure — row stays unpublished for retry.
    #[error(transparent)]
    Sink(#[from] MessagingError),
}

/// Runs until the process exits: poll unpublished outbox rows and deliver.
pub async fn run_publisher(
    pool: StorePool,
    metrics: Arc<OutboxMetrics>,
    sink: Arc<dyn EventSink>,
    interval: Duration,
) {
    loop {
        if let Err(err) = publish_once_with(&pool, &metrics, sink.as_ref()).await {
            metrics.publish_errors.fetch_add(1, Ordering::Relaxed);
            eprintln!("shinrai-order-gateway: outbox publish error: {err}");
        }
        tokio::time::sleep(interval).await;
    }
}

/// One poll cycle (also used by tests). Uses [`LogSink`] when no sink is passed
/// via [`publish_once_with`].
///
/// # Errors
///
/// Returns store or sink errors.
pub async fn publish_once(
    pool: &StorePool,
    metrics: &OutboxMetrics,
) -> Result<usize, OutboxPublishError> {
    publish_once_with(pool, metrics, &LogSink).await
}

/// One poll cycle with an explicit sink.
///
/// # Errors
///
/// Returns store or sink errors. On sink failure the current event is not
/// marked published so a later poll can retry (at-least-once).
pub async fn publish_once_with(
    pool: &StorePool,
    metrics: &OutboxMetrics,
    sink: &dyn EventSink,
) -> Result<usize, OutboxPublishError> {
    let events = claim_unpublished(pool, 64).await?;
    let mut n = 0;
    for event in events {
        deliver(pool, metrics, sink, &event).await?;
        n += 1;
    }
    Ok(n)
}

async fn deliver(
    pool: &StorePool,
    metrics: &OutboxMetrics,
    sink: &dyn EventSink,
    event: &OutboxEvent,
) -> Result<(), OutboxPublishError> {
    let envelope = EventEnvelope {
        event_id: event.id,
        topic: event.topic.clone(),
        payload: event.payload.clone(),
    };
    // Deliver before mark_published so a sink failure leaves the row for retry.
    sink.publish(&envelope).await?;
    mark_published(pool, event.id).await?;
    metrics.published.fetch_add(1, Ordering::Relaxed);
    Ok(())
}
