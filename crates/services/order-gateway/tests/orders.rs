//! Paper order submit via authenticated HTTP.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::json;
use shinrai_order_gateway::{router, AppState};
use tower::ServiceExt;

#[tokio::test]
async fn submit_buy_fills() {
    let app = router(AppState::for_test("paper-tok", "trader", 1, 10_000));

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/orders?token=paper-tok")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "client_order_id": "ord-1",
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
        .expect("resp");
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("bytes");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(json["status"], "Filled");
    assert_eq!(json["symbol"], "AAPL");
    assert_eq!(json["cum_qty"], 10);

    let order_id = json["id"].as_u64().expect("id");
    let get = app
        .oneshot(
            Request::builder()
                .uri(format!("/v1/orders/{order_id}?token=paper-tok"))
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("get");
    assert_eq!(get.status(), StatusCode::OK);
}

#[tokio::test]
async fn risk_rejects_insufficient_funds() {
    let app = router(AppState::for_test("paper-tok", "trader", 1, 1));

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/orders?token=paper-tok")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "client_order_id": "ord-poor",
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
        .expect("resp");
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("bytes");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(json["code"], "insufficient_buying_power");
}

#[tokio::test]
async fn client_credentials_flow() {
    let app = router(AppState::for_test_client(
        "dev", "s3cret", "trader", 1, 10_000,
    ));

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

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/orders")
                .header("authorization", format!("Bearer {access}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "client_order_id": "cc-1",
                        "symbol": "AAPL",
                        "side": "Buy",
                        "qty": 5,
                        "price": 10000
                    })
                    .to_string(),
                ))
                .expect("req"),
        )
        .await
        .expect("order");
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn idempotent_client_order_id() {
    let app = router(AppState::for_test("paper-tok", "trader", 1, 10_000));
    let body = json!({
        "client_order_id": "dup",
        "symbol": "AAPL",
        "side": "Buy",
        "qty": 1,
        "price": 10000
    })
    .to_string();

    let first = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/orders?token=paper-tok")
                .header("content-type", "application/json")
                .body(Body::from(body.clone()))
                .expect("req"),
        )
        .await
        .expect("first");
    assert_eq!(first.status(), StatusCode::OK);
    let id1 = {
        let bytes = axum::body::to_bytes(first.into_body(), usize::MAX)
            .await
            .expect("bytes");
        serde_json::from_slice::<serde_json::Value>(&bytes).expect("json")["id"]
            .as_u64()
            .expect("id")
    };

    let second = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/orders?token=paper-tok")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .expect("req"),
        )
        .await
        .expect("second");
    assert_eq!(second.status(), StatusCode::OK);
    let id2 = {
        let bytes = axum::body::to_bytes(second.into_body(), usize::MAX)
            .await
            .expect("bytes");
        serde_json::from_slice::<serde_json::Value>(&bytes).expect("json")["id"]
            .as_u64()
            .expect("id")
    };
    assert_eq!(id1, id2);
}
