//! Paper trader UI is served without auth (APIs still require Bearer).

use axum::body::Body;
use axum::http::{Request, StatusCode};
use shinrai_order_gateway::{router, AppState};
use tower::ServiceExt;

#[tokio::test]
async fn ui_serves_html() {
    let app = router(AppState::for_test("ui-tok", "trader", 1, 10_000));
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/ui")
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("resp");
    assert_eq!(resp.status(), StatusCode::OK);
    let ct = resp
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(ct.contains("text/html"), "ct={ct}");
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("bytes");
    let html = String::from_utf8_lossy(&body);
    assert!(html.contains("paper trader"));
    assert!(html.contains("/v1/orders"));
    assert!(html.contains("Quick start"));
}