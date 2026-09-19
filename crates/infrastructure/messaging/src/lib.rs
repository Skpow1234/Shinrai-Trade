//! Event bus sinks for the transactional outbox publisher.
//!
//! Default is [`LogSink`] (tracing only). When `SHINRAI_NATS_URL` is set, the
//! order gateway connects a [`NatsSink`] and publishes before marking outbox
//! rows published (at-least-once delivery).

#![forbid(unsafe_code)]

mod nats;
mod sink;

pub use nats::{connect_nats, NatsSink};
pub use sink::{EventEnvelope, EventSink, LogSink, MessagingError, SinkKind};
