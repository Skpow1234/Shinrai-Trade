//! NATS core publish sink.

use std::future::Future;
use std::pin::Pin;

use async_nats::Client;
use bytes::Bytes;
use serde_json::json;

use crate::sink::{EventEnvelope, EventSink, MessagingError, SinkKind};

/// Publishes outbox envelopes to NATS subjects `{prefix}.{topic}`.
#[derive(Debug, Clone)]
pub struct NatsSink {
    client: Client,
    subject_prefix: String,
}

impl NatsSink {
    /// Wraps an existing connected client.
    #[must_use]
    pub fn new(client: Client, subject_prefix: impl Into<String>) -> Self {
        Self {
            client,
            subject_prefix: subject_prefix.into(),
        }
    }

    fn subject_for(&self, topic: &str) -> String {
        let prefix = self.subject_prefix.trim_end_matches('.');
        if prefix.is_empty() {
            topic.to_string()
        } else {
            format!("{prefix}.{topic}")
        }
    }
}

impl EventSink for NatsSink {
    fn kind(&self) -> SinkKind {
        SinkKind::Nats
    }

    fn publish<'a>(
        &'a self,
        event: &'a EventEnvelope,
    ) -> Pin<Box<dyn Future<Output = Result<(), MessagingError>> + Send + 'a>> {
        Box::pin(async move {
            let subject = self.subject_for(&event.topic);
            let body = json!({
                "event_id": event.event_id,
                "topic": event.topic,
                "payload": event.payload,
            });
            let bytes = Bytes::from(serde_json::to_vec(&body)?);
            self.client
                .publish(subject.clone(), bytes)
                .await
                .map_err(|e| MessagingError::Nats(e.to_string()))?;
            // Best-effort flush so a process crash soon after publish is less
            // likely to lose the last few messages in the client buffer.
            self.client
                .flush()
                .await
                .map_err(|e| MessagingError::Nats(e.to_string()))?;
            tracing::info!(
                target: "shinrai_outbox",
                event_id = event.event_id,
                topic = %event.topic,
                subject = %subject,
                "outbox.delivered"
            );
            Ok(())
        })
    }
}

/// Connects to NATS (`nats://…`). Empty / unset URL should not call this.
///
/// # Errors
///
/// Returns [`MessagingError::Nats`] when the connection fails.
pub async fn connect_nats(
    url: &str,
    subject_prefix: impl Into<String>,
) -> Result<NatsSink, MessagingError> {
    let client = async_nats::connect(url)
        .await
        .map_err(|e| MessagingError::Nats(e.to_string()))?;
    Ok(NatsSink::new(client, subject_prefix))
}
