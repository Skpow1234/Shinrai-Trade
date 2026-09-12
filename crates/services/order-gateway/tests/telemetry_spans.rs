//! Order path remains healthy with `TraceLayer` enabled (spans are side-effect free).

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::json;
use shinrai_order_gateway::{router, AppState};
use tower::ServiceExt;

#[tokio::test]
async fn submit_and_cancel_with_http_trace_layer() {
    let app = router(AppState::for_test("otel-tok", "trader", 1, 10_000));

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/orders?token=otel-tok")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "client_order_id": "otel-1",
                        "symbol": "AAPL",
                        "side": "Buy",
                        "qty": 1,
                        "price": 10000
                    })
                    .to_string(),
                ))
                .expect("req"),
        )
        .await
        .expect("resp");
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("bytes");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json");
    let order_id = json["id"].as_u64().expect("id");

    let cancel = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/orders/{order_id}/cancel?token=otel-tok"))
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("cancel");
    // Immediate fill may already be terminal; cancel still returns 200 or conflict-safe path.
    assert!(
        cancel.status() == StatusCode::OK || cancel.status() == StatusCode::CONFLICT,
        "unexpected cancel status {}",
        cancel.status()
    );
}
