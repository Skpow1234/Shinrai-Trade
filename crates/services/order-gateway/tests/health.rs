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
}
