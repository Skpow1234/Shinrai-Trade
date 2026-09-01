//! Portfolio, audit, and reconciliation HTTP.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::json;
use shinrai_order_gateway::{router, AppState};
use tower::ServiceExt;

#[tokio::test]
async fn portfolio_and_audit_after_order() {
    let app = router(AppState::for_test("paper-tok", "trader", 1, 10_000));

    let order = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/orders?token=paper-tok")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "client_order_id": "pf-1",
                        "symbol": "AAPL",
                        "side": "Buy",
                        "qty": 10,
                        "price": 10000
                    })
                    .to_string(),
                ))
                .expect("req"),
        )
        .await
        .expect("order");
    assert_eq!(order.status(), StatusCode::OK);

    let portfolio = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/portfolio?token=paper-tok&marks=AAPL:11000")
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("pf");
    assert_eq!(portfolio.status(), StatusCode::OK);
    let body = axum::body::to_bytes(portfolio.into_body(), usize::MAX)
        .await
        .expect("bytes");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(json["positions"][0]["lots"], 10);
    assert!(json["total_unrealized_pnl_minor"]
        .as_i64()
        .is_some_and(|v| v > 0));

    let audit = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/audit?token=paper-tok")
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("audit");
    assert_eq!(audit.status(), StatusCode::OK);
    let body = axum::body::to_bytes(audit.into_body(), usize::MAX)
        .await
        .expect("bytes");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json");
    assert!(json["records"].as_array().is_some_and(|a| !a.is_empty()));

    let recon = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/reconciliation?token=paper-tok")
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("recon");
    assert_eq!(recon.status(), StatusCode::OK);
    let body = axum::body::to_bytes(recon.into_body(), usize::MAX)
        .await
        .expect("bytes");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(json["ok"], true);

    let metrics = app
        .oneshot(
            Request::builder()
                .uri("/v1/metrics")
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("metrics");
    assert_eq!(metrics.status(), StatusCode::OK);
    let body = axum::body::to_bytes(metrics.into_body(), usize::MAX)
        .await
        .expect("bytes");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(json["orders_submitted"], 1);
    assert_eq!(json["orders_accepted"], 1);
}

#[tokio::test]
async fn stored_marks_value_portfolio_without_manual_marks() {
    let app = router(AppState::for_test("paper-tok", "trader", 1, 10_000));

    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/orders?token=paper-tok")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "client_order_id": "mark-fill",
                        "symbol": "AAPL",
                        "side": "Buy",
                        "qty": 10,
                        "price": 10000
                    })
                    .to_string(),
                ))
                .expect("req"),
        )
        .await
        .expect("order");

    let portfolio = app
        .oneshot(
            Request::builder()
                .uri("/v1/portfolio?token=paper-tok&use_stored_marks=1")
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("pf");
    assert_eq!(portfolio.status(), StatusCode::OK);
    let body = axum::body::to_bytes(portfolio.into_body(), usize::MAX)
        .await
        .expect("bytes");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(json["positions"][0]["mark_scaled"], 10_000);
    assert!(json["total_cost_basis_minor"].as_i64().is_some());
}
