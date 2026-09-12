//! Ops metrics and stuck-order HTTP coverage.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::json;
use shinrai_order_gateway::{router, AppState};
use tower::ServiceExt;

#[tokio::test]
async fn metrics_includes_oms_ops_fields() {
    let app = router(AppState::for_test("ops-tok", "trader", 1, 10_000));

    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/orders?token=ops-tok")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "client_order_id": "ops-1",
                        "symbol": "AAPL",
                        "side": "Buy",
                        "qty": 2,
                        "price": 10000
                    })
                    .to_string(),
                ))
                .expect("req"),
        )
        .await
        .expect("order");

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/v1/metrics")
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("metrics");
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("bytes");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(json["orders_submitted"], 1);
    assert_eq!(json["orders_accepted"], 1);
    assert_eq!(json["orders_by_status"]["Filled"], 1);
    assert_eq!(json["stuck_count"], 0);
    assert_eq!(json["ledger_trial_balance_ok"], true);
    assert_eq!(json["reconciliation_ok"], true);
    assert_eq!(json["store_enabled"], false);
}

#[tokio::test]
async fn stuck_orders_endpoint_lists_pending_new() {
    let app = router(AppState::for_test_with_stuck_pending(
        "ops-tok", "trader", 1, 99,
    ));
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/v1/ops/stuck-orders?max_age_secs=0")
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("stuck");
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("bytes");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(json["stuck_count"], 1);
    assert_eq!(json["stuck_orders"][0]["order_id"], 99);
    assert_eq!(json["stuck_orders"][0]["status"], "PendingNew");
}

#[tokio::test]
async fn ops_dashboard_html() {
    let app = router(AppState::for_test("ops-tok", "trader", 1, 10_000));
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/v1/ops")
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("ops");
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("bytes");
    let html = String::from_utf8(body.to_vec()).expect("utf8");
    assert!(html.contains("Shinrai"));
    assert!(html.contains("/v1/metrics"));
}
