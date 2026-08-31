//! Auth token issue / refresh / revoke.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::json;
use shinrai_md_gateway::{router, AppState};
use tower::ServiceExt;

#[tokio::test]
async fn client_credentials_then_bars() {
    let app = router(AppState::for_test_client("dev", "s3cret", "alice"));

    let issued = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/auth/token")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "grant_type": "client_credentials",
                        "client_id": "dev",
                        "client_secret": "s3cret"
                    })
                    .to_string(),
                ))
                .expect("req"),
        )
        .await
        .expect("token");
    assert_eq!(issued.status(), StatusCode::OK);
    let body = axum::body::to_bytes(issued.into_body(), usize::MAX)
        .await
        .expect("bytes");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json");
    let access = json["access_token"].as_str().expect("access");
    assert_eq!(json["token_type"], "Bearer");
    assert!(json["expires_in"]
        .as_u64()
        .is_some_and(|e| e > 0 && e <= 60));

    let bars = app
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/v1/bars?symbol=BTC-USD&interval=1m&limit=1&token={access}"
                ))
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("bars");
    assert_eq!(bars.status(), StatusCode::OK);
}

#[tokio::test]
async fn refresh_rotation_and_reuse_fails() {
    let app = router(AppState::for_test_client("dev", "s3cret", "alice"));

    let issued = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/auth/token")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "grant_type": "client_credentials",
                        "client_id": "dev",
                        "client_secret": "s3cret"
                    })
                    .to_string(),
                ))
                .expect("req"),
        )
        .await
        .expect("token");
    let body = axum::body::to_bytes(issued.into_body(), usize::MAX)
        .await
        .expect("bytes");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json");
    let refresh = json["refresh_token"].as_str().expect("rt").to_owned();

    let rotated = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/auth/token")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "grant_type": "refresh_token",
                        "refresh_token": refresh
                    })
                    .to_string(),
                ))
                .expect("req"),
        )
        .await
        .expect("refresh");
    assert_eq!(rotated.status(), StatusCode::OK);

    let reuse = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/auth/token")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "grant_type": "refresh_token",
                        "refresh_token": refresh
                    })
                    .to_string(),
                ))
                .expect("req"),
        )
        .await
        .expect("reuse");
    assert_eq!(reuse.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn revoke_rejects_access() {
    let app = router(AppState::for_test_client("dev", "s3cret", "alice"));

    let issued = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/auth/token")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "grant_type": "client_credentials",
                        "client_id": "dev",
                        "client_secret": "s3cret"
                    })
                    .to_string(),
                ))
                .expect("req"),
        )
        .await
        .expect("token");
    let body = axum::body::to_bytes(issued.into_body(), usize::MAX)
        .await
        .expect("bytes");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json");
    let access = json["access_token"].as_str().expect("access").to_owned();

    let rev = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/auth/revoke")
                .header("content-type", "application/json")
                .body(Body::from(json!({ "token": access }).to_string()))
                .expect("req"),
        )
        .await
        .expect("revoke");
    assert_eq!(rev.status(), StatusCode::OK);

    let bars = app
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/v1/bars?symbol=BTC-USD&interval=1m&token={access}"
                ))
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("bars");
    assert_eq!(bars.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn bad_secret_is_unauthorized() {
    let app = router(AppState::for_test_client("dev", "s3cret", "alice"));
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/auth/token")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "grant_type": "client_credentials",
                        "client_id": "dev",
                        "client_secret": "nope"
                    })
                    .to_string(),
                ))
                .expect("req"),
        )
        .await
        .expect("token");
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}
