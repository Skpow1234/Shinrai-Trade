//! Historical REST API tests.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use shinrai_md_gateway::{router, AppState};
use tower::ServiceExt;

#[tokio::test]
async fn bars_requires_auth() {
    let app = router(AppState::for_test("t", "alice"));
    let res = app
        .oneshot(
            Request::builder()
                .uri("/v1/bars?symbol=BTC-USD&interval=1m")
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("oneshot");
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn bars_and_trades_paginate() {
    let app = router(AppState::for_test("dev", "alice"));

    let bars = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/bars?symbol=BTC-USD&interval=1m&limit=2&token=dev")
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("bars");
    assert_eq!(bars.status(), StatusCode::OK);
    let body = axum::body::to_bytes(bars.into_body(), usize::MAX)
        .await
        .expect("bytes");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(json["type"], "bars");
    assert!(json["bars"].as_array().is_some_and(|a| !a.is_empty()));

    let trades = app
        .oneshot(
            Request::builder()
                .uri("/v1/trades?symbol=BTC-USD&limit=5&token=dev")
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("trades");
    assert_eq!(trades.status(), StatusCode::OK);
    let body = axum::body::to_bytes(trades.into_body(), usize::MAX)
        .await
        .expect("bytes");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(json["type"], "trades");
    assert_eq!(json["trades"].as_array().map(Vec::len), Some(5));
}

#[tokio::test]
async fn unknown_symbol_is_not_found() {
    let app = router(AppState::for_test("dev", "alice"));
    let res = app
        .oneshot(
            Request::builder()
                .uri("/v1/trades?symbol=NOPE&token=dev")
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("oneshot");
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn quote_returns_last_trade() {
    let app = router(AppState::for_test("dev", "alice"));
    let res = app
        .oneshot(
            Request::builder()
                .uri("/v1/quotes?symbol=BTC-USD&token=dev")
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("quote");
    assert_eq!(res.status(), StatusCode::OK);
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .expect("bytes");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(json["type"], "quote");
    assert!(json["price_scaled"].as_i64().is_some_and(|p| p > 0));
}
