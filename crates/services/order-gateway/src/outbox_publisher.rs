//! Background outbox publisher (log sink + inbox dedup).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use shinrai_store::{
    claim_unpublished, mark_published, try_claim_inbox, OutboxEvent, StoreError, StorePool,
};

const CONSUMER: &str = "order-gateway.log";

/// Shared counters for ops metrics.
#[derive(Debug, Default)]
pub struct OutboxMetrics {
    published: AtomicU64,
    publish_errors: AtomicU64,
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

/// Runs until the process exits: poll unpublished outbox rows and deliver.
pub async fn run_publisher(pool: StorePool, metrics: Arc<OutboxMetrics>, interval: Duration) {
    loop {
        if let Err(err) = publish_once(&pool, &metrics).await {
            metrics.publish_errors.fetch_add(1, Ordering::Relaxed);
            eprintln!("shinrai-order-gateway: outbox publish error: {err}");
        }
        tokio::time::sleep(interval).await;
    }
}

/// One poll cycle (also used by tests).
///
/// # Errors
///
/// Returns store errors.
pub async fn publish_once(pool: &StorePool, metrics: &OutboxMetrics) -> Result<usize, StoreError> {
    let events = claim_unpublished(pool, 64).await?;
    let mut n = 0;
    for event in events {
        deliver(pool, metrics, &event).await?;
        n += 1;
    }
    Ok(n)
}

async fn deliver(
    pool: &StorePool,
    metrics: &OutboxMetrics,
    event: &OutboxEvent,
) -> Result<(), StoreError> {
    if !try_claim_inbox(pool, CONSUMER, event.id).await? {
        metrics.duplicates_skipped.fetch_add(1, Ordering::Relaxed);
        mark_published(pool, event.id).await?;
        return Ok(());
    }
    // Log sink — stand-in until Kafka/NATS. Payload must not contain secrets.
    tracing::info!(
        target: "shinrai_outbox",
        event_id = event.id,
        topic = %event.topic,
        payload = %event.payload,
        "outbox.delivered"
    );
    mark_published(pool, event.id).await?;
    metrics.published.fetch_add(1, Ordering::Relaxed);
    Ok(())
}
