//! Health / readiness endpoints.

use tower::ServiceExt;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use shinrai_order_gateway::{router, AppState};

#[tokio::test]
async fn health_ok() {
    let app = router(AppState::for_test("tok", "alice", 1, 10_000));
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("resp");
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body");
    let v: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
    assert_eq!(v["status"], "ok");
    assert_eq!(v["ha"]["enabled"], false);
    assert_eq!(v["ha"]["leader"], true);
}

#[tokio::test]
async fn ready_ok_when_leader() {
    let app = router(AppState::for_test("tok", "alice", 1, 10_000));
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/ready")
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("resp");
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body");
    let v: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
    assert_eq!(v["ready"], true);
    assert_eq!(v["hydrate_complete"], true);
    assert!(v["reasons"].as_array().expect("reasons").is_empty());
}

#[tokio::test]
async fn ready_503_when_promoting() {
    let state = AppState::for_test("tok", "alice", 1, 10_000);
    state.set_oms_promoting(true);
    let app = router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/ready")
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("resp");
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body");
    let v: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
    assert_eq!(v["ready"], false);
    let reasons = v["reasons"].as_array().expect("reasons");
    assert!(reasons.iter().any(|r| r == "promoting"));
}

#[tokio::test]
async fn ready_503_when_persist_degraded() {
    let state = AppState::for_test("tok", "alice", 1, 10_000);
    state.mark_persist_degraded_for_test();
    let app = router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/ready")
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("resp");
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body");
    let v: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
    assert_eq!(v["ready"], false);
    assert_eq!(v["persist_degraded"], true);
    // Liveness still OK.
    let health = router(AppState::for_test("tok2", "alice", 1, 10_000))
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("health");
    assert_eq!(health.status(), StatusCode::OK);
}
