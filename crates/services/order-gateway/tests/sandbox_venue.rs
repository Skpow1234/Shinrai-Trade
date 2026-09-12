//! Sandbox venue via order gateway.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::json;
use shinrai_order_gateway::{router, AppState};
use tower::ServiceExt;

#[tokio::test]
async fn sandbox_submit_fills() {
    let app = router(AppState::for_test_sandbox("sbx-tok", "trader", 1, 10_000));

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/orders?token=sbx-tok")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "client_order_id": "sbx-1",
                        "symbol": "AAPL",
                        "side": "Buy",
                        "qty": 4,
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
    assert_eq!(json["status"], "Filled");
    assert_eq!(json["cum_qty"], 4);

    let metrics = app
        .oneshot(
            Request::builder()
                .uri("/v1/metrics")
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("metrics");
    let body = axum::body::to_bytes(metrics.into_body(), usize::MAX)
        .await
        .expect("bytes");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(json["venue"], "sandbox");
}
