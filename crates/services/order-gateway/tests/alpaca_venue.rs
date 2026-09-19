//! Local Alpaca paper venue via order gateway.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::json;
use shinrai_order_gateway::{router, AppState};
use tower::ServiceExt;

#[tokio::test]
async fn alpaca_local_submit_fills_and_eod() {
    let app =
        router(AppState::for_test_alpaca("alp-tok", "trader", 1, 10_000).with_ops_token("ops-alp"));

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/orders?token=alp-tok")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "client_order_id": "alp-1",
                        "symbol": "AAPL",
                        "side": "Buy",
                        "qty": 3,
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
    assert_eq!(json["cum_qty"], 3);

    let metrics = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/metrics?ops_token=ops-alp")
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("metrics");
    let body = axum::body::to_bytes(metrics.into_body(), usize::MAX)
        .await
        .expect("bytes");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(json["venue"], "alpaca");

    let eod = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/ops/reconciliation/eod?ops_token=ops-alp")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "source": "alpaca",
                        "account_id": 1
                    })
                    .to_string(),
                ))
                .expect("req"),
        )
        .await
        .expect("eod");
    assert_eq!(eod.status(), StatusCode::OK);
    let body = axum::body::to_bytes(eod.into_body(), usize::MAX)
        .await
        .expect("bytes");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(json["source"], "alpaca");
    assert_eq!(
        json["ok"], true,
        "alpaca position EOD should match after fill: {json}"
    );
}
