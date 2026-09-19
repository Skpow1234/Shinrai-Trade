//! Log sink unit smoke.

use shinrai_messaging::{EventEnvelope, EventSink, LogSink, SinkKind};
use serde_json::json;

#[tokio::test]
async fn log_sink_publishes_ok() {
    let sink = LogSink;
    assert_eq!(sink.kind(), SinkKind::Log);
    sink.publish(&EventEnvelope {
        event_id: 1,
        topic: "ledger.posted".into(),
        payload: json!({"kind": "test"}),
    })
    .await
    .expect("log sink");
}
