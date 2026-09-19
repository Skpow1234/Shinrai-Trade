//! Phase 5 paper fund movements.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::json;
use shinrai_order_gateway::{router, AppState};
use tower::ServiceExt;

#[tokio::test]
async fn deposit_withdraw_and_balances() {
    let app = router(AppState::for_test("funds-tok", "trader", 1, 1_000));

    let bal = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/accounts/balances?token=funds-tok")
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("resp");
    assert_eq!(bal.status(), StatusCode::OK);
    let body = axum::body::to_bytes(bal.into_body(), usize::MAX)
        .await
        .expect("bytes");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(json["mode"], "paper");
    assert_eq!(json["balances"][0]["available_minor"], 100_000); // $1000

    let dep = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/accounts/deposit?token=funds-tok")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "idempotency_key": "dep-1",
                        "amount_minor": 25_000,
                        "currency": "USD"
                    })
                    .to_string(),
                ))
                .expect("req"),
        )
        .await
        .expect("resp");
    assert_eq!(dep.status(), StatusCode::OK);
    let body = axum::body::to_bytes(dep.into_body(), usize::MAX)
        .await
        .expect("bytes");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(json["available_minor"], 125_000);

    // Idempotent replay
    let dep2 = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/accounts/deposit?token=funds-tok")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "idempotency_key": "dep-1",
                        "amount_minor": 25_000
                    })
                    .to_string(),
                ))
                .expect("req"),
        )
        .await
        .expect("resp");
    assert_eq!(dep2.status(), StatusCode::OK);
    let body = axum::body::to_bytes(dep2.into_body(), usize::MAX)
        .await
        .expect("bytes");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(json["available_minor"], 125_000);

    let wd = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/accounts/withdraw?token=funds-tok")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "idempotency_key": "wd-1",
                        "amount_minor": 10_000
                    })
                    .to_string(),
                ))
                .expect("req"),
        )
        .await
        .expect("resp");
    assert_eq!(wd.status(), StatusCode::OK);
    let body = axum::body::to_bytes(wd.into_body(), usize::MAX)
        .await
        .expect("bytes");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(json["available_minor"], 115_000);

    let over = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/accounts/withdraw?token=funds-tok")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "idempotency_key": "wd-too-much",
                        "amount_minor": 999_999_999
                    })
                    .to_string(),
                ))
                .expect("req"),
        )
        .await
        .expect("resp");
    assert_eq!(over.status(), StatusCode::CONFLICT);
}
