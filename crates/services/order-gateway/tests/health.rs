//! Health endpoint.

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
