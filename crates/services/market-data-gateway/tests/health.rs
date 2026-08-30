//! Gateway HTTP and WebSocket smoke tests (no live venue).

use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use futures_util::{SinkExt, StreamExt};
use shinrai_md_gateway::{router, AppState};
use tokio::net::TcpListener;
use tower::ServiceExt;

async fn spawn_gateway(state: AppState) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        axum::serve(listener, router(state)).await.expect("serve");
    });
    addr.to_string()
}

#[tokio::test]
async fn health_ok() {
    let app = router(AppState::for_test("t", "alice"));
    let res = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("oneshot");
    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test]
async fn ws_rejects_missing_token() {
    let addr = spawn_gateway(AppState::for_test("t", "alice")).await;
    let url = format!("ws://{addr}/v1/ws");
    let mut last = None;
    for _ in 0..50 {
        match tokio_tungstenite::connect_async(&url).await {
            Ok(_) => panic!("unauthenticated connect must fail"),
            Err(tokio_tungstenite::tungstenite::Error::Http(resp)) => {
                assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
                return;
            }
            Err(err) => {
                last = Some(err);
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
    }
    panic!("expected HTTP 401, last error: {last:?}");
}

#[tokio::test]
async fn ws_subscribe_btc_usd() {
    let addr = spawn_gateway(AppState::for_test("test-token", "alice")).await;
    let url = format!("ws://{addr}/v1/ws?token=test-token");
    let mut attempt = 0;
    let (mut ws, _) = loop {
        match tokio_tungstenite::connect_async(&url).await {
            Ok(pair) => break pair,
            Err(err) => {
                attempt += 1;
                assert!(attempt <= 50, "connect: {err}");
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
    };
    ws.send(tokio_tungstenite::tungstenite::Message::text(
        r#"{"type":"subscribe","symbol":"BTC-USD"}"#,
    ))
    .await
    .expect("send");

    let msg = tokio::time::timeout(Duration::from_secs(2), ws.next())
        .await
        .expect("timeout")
        .expect("next")
        .expect("frame");
    let text = msg.into_text().expect("text");
    assert!(text.contains("subscribed"), "{text}");
    assert!(text.contains('3'), "{text}");
}
