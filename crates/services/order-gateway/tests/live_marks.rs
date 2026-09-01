//! Live marks integration: portfolio fetches quotes from a running MD gateway.

use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::json;
use shinrai_md_gateway::{router as md_router, AppState as MdAppState};
use shinrai_order_gateway::{router, AppState};
use tokio::net::TcpListener;
use tower::ServiceExt;

async fn spawn_md_gateway(token: &str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let base = format!("http://{}", listener.local_addr().expect("addr"));
    let state = MdAppState::for_test(token, "alice");
    tokio::spawn(async move {
        axum::serve(listener, md_router(state))
            .await
            .expect("serve md gateway");
    });
    wait_for_health(&base).await;
    base
}

async fn wait_for_health(base: &str) {
    let client = reqwest::Client::new();
    for _ in 0..50 {
        if client
            .get(format!("{base}/health"))
            .send()
            .await
            .is_ok_and(|r| r.status().is_success())
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("md gateway not ready");
}

async fn fetch_md_quote(base: &str, token: &str, symbol: &str) -> i64 {
    let resp = reqwest::Client::new()
        .get(format!("{base}/v1/quotes"))
        .query(&[("symbol", symbol), ("token", token)])
        .send()
        .await
        .expect("quote request");
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = resp.json().await.expect("quote json");
    body["price_scaled"].as_i64().expect("price_scaled")
}

#[tokio::test]
async fn live_marks_from_md_gateway() {
    let md_token = "md-tok";
    let md_base = spawn_md_gateway(md_token).await;
    let live_mark = fetch_md_quote(&md_base, md_token, "BTC-USD").await;
    assert!(live_mark > 0);

    let og = router(AppState::for_test_with_md(
        "paper-tok",
        "trader",
        1,
        10_000,
        &md_base,
        md_token,
    ));

    let order = og
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/orders?token=paper-tok")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "client_order_id": "live-mark-1",
                        "symbol": "BTC-USD",
                        "side": "Buy",
                        "qty": 1,
                        "price": 10_000
                    })
                    .to_string(),
                ))
                .expect("req"),
        )
        .await
        .expect("order");
    assert_eq!(order.status(), StatusCode::OK);

    let without_live = og
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/portfolio?token=paper-tok&use_stored_marks=0&use_live_marks=0")
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("portfolio without live");
    assert_eq!(without_live.status(), StatusCode::OK);
    let body = axum::body::to_bytes(without_live.into_body(), usize::MAX)
        .await
        .expect("bytes");
    let without: serde_json::Value = serde_json::from_slice(&body).expect("json");
    assert!(without["positions"][0]["mark_scaled"].is_null());
    assert!(without["total_unrealized_pnl_minor"].is_null());

    let portfolio = og
        .oneshot(
            Request::builder()
                .uri("/v1/portfolio?token=paper-tok&use_live_marks=1&use_stored_marks=0")
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("portfolio");
    assert_eq!(portfolio.status(), StatusCode::OK);
    let body = axum::body::to_bytes(portfolio.into_body(), usize::MAX)
        .await
        .expect("bytes");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json");

    assert_eq!(json["positions"][0]["symbol"], "BTC-USD");
    assert_eq!(json["positions"][0]["mark_scaled"], live_mark);
    assert_eq!(json["positions"][0]["avg_cost_scaled"], 10_000);
    assert!(json["total_unrealized_pnl_minor"].as_i64().is_some());
}

#[tokio::test]
async fn live_marks_requires_md_url() {
    let app = router(AppState::for_test("paper-tok", "trader", 1, 10_000));

    let res = app
        .oneshot(
            Request::builder()
                .uri("/v1/portfolio?token=paper-tok&use_live_marks=1")
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("portfolio");
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .expect("bytes");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("json");
    assert_eq!(json["code"], "md_url_not_configured");
}
