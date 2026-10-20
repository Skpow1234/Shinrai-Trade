//! Live Alpaca stock market-data I/O (sockets stay in the gateway).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use shinrai_instruments::{aapl, ExternalId, InstrumentMaster};
use shinrai_market_data::{HistoricalArchive, MdKind};
use shinrai_md_fanout::{FanoutHub, TokenAuth};
use shinrai_md_protocol::{
    AlpacaMarkets, FeedCommand, FeedSupervisor, IngestOutcome, MarketDataVendor, SupervisorEvent,
};
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::app::unix_logical_now;
use crate::coinbase_feed::dispatch_event;

const SLA_SECS: u64 = 30;

type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Spawns Alpaca IEX (or SIP) stream for the given equity product ids.
pub(crate) fn spawn(
    hub: Arc<Mutex<FanoutHub<TokenAuth>>>,
    history: Arc<Mutex<HistoricalArchive>>,
    master: InstrumentMaster,
    api_key: String,
    api_secret: String,
    feed: String,
    products: Vec<String>,
) {
    tokio::spawn(async move {
        if let Err(err) = run_loop(hub, history, master, api_key, api_secret, feed, products).await
        {
            eprintln!("shinrai-md-gateway: alpaca feed stopped: {err}");
        }
    });
}

async fn run_loop(
    hub: Arc<Mutex<FanoutHub<TokenAuth>>>,
    history: Arc<Mutex<HistoricalArchive>>,
    master: InstrumentMaster,
    api_key: String,
    api_secret: String,
    feed: String,
    products: Vec<String>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let vendor = if feed.eq_ignore_ascii_case("sip") {
        AlpacaMarkets::sip(api_key, api_secret)
    } else {
        AlpacaMarkets::iex(api_key, api_secret)
    };
    let http = reqwest::Client::builder()
        .user_agent("shinrai-md-gateway/0.1")
        .timeout(Duration::from_secs(10))
        .build()?;

    let mut delay = Duration::from_secs(1);
    loop {
        match session(&vendor, &http, &hub, &history, &master, &products).await {
            Ok(()) => {
                eprintln!("shinrai-md-gateway: alpaca session ended; reconnecting");
                delay = Duration::from_secs(1);
            }
            Err(err) => {
                eprintln!("shinrai-md-gateway: alpaca session error: {err}; retry in {delay:?}");
            }
        }
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(Duration::from_secs(32));
    }
}

async fn session(
    vendor: &AlpacaMarkets,
    http: &reqwest::Client,
    hub: &Arc<Mutex<FanoutHub<TokenAuth>>>,
    history: &Arc<Mutex<HistoricalArchive>>,
    master: &InstrumentMaster,
    products: &[String],
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut supervisor = FeedSupervisor::new(SLA_SECS);
    for product in products {
        let alias = ExternalId::ticker(product)?;
        let id = master.resolve_alias(&alias)?;
        supervisor.watch(id, product.clone());
    }
    if products.is_empty() {
        supervisor.watch(aapl().id(), "AAPL");
    }

    let url = vendor.stream_url();
    eprintln!("shinrai-md-gateway: alpaca connecting → {url}");
    let (mut ws, _) = connect_async(&url).await?;

    // Connected banner, then auth, then wait for auth success.
    wait_control(&mut ws, "connected").await?;
    let auth = String::from_utf8(vendor.auth_message())?;
    ws.send(Message::Text(auth.into())).await?;
    wait_control(&mut ws, "authenticated").await?;

    let now = unix_logical_now();
    let commands = supervisor.on_connected(vendor, now);
    execute_commands(
        &mut ws,
        http,
        vendor,
        master,
        &mut supervisor,
        hub,
        history,
        commands,
    )
    .await?;

    let mut clock = tokio::time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            _ = clock.tick() => {
                let now = unix_logical_now();
                for event in supervisor.on_clock(now) {
                    dispatch_event(hub, history, &event);
                }
            }
            msg = ws.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        let now = unix_logical_now();
                        ingest_text(
                            vendor,
                            master,
                            &mut supervisor,
                            hub,
                            history,
                            http,
                            &mut ws,
                            now,
                            text.as_str(),
                        )
                        .await?;
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        ws.send(Message::Pong(payload)).await?;
                    }
                    None | Some(Ok(Message::Close(_)) | Err(_)) => {
                        let commands = supervisor.on_disconnect();
                        dispatch_event(
                            hub,
                            history,
                            &SupervisorEvent::Stale {
                                instrument_id: aapl().id(),
                                silent_for: SLA_SECS.saturating_add(1),
                            },
                        );
                        let _ = execute_commands(
                            &mut ws, http, vendor, master, &mut supervisor, hub, history, commands,
                        )
                        .await;
                        return Ok(());
                    }
                    Some(Ok(_)) => {}
                }
            }
        }
    }
}

async fn wait_control(
    ws: &mut WsStream,
    needle: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_secs(5), ws.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => {
                if text.contains(needle) {
                    return Ok(());
                }
                // Keep draining until auth/connected.
            }
            Ok(Some(Ok(Message::Ping(p)))) => {
                ws.send(Message::Pong(p)).await?;
            }
            Ok(None | Some(Ok(Message::Close(_)) | Err(_))) => {
                return Err("alpaca ws closed during auth".into());
            }
            Ok(Some(Ok(_))) => {}
            Err(_) => return Err("alpaca auth timeout".into()),
        }
    }
    Err("alpaca auth deadline exceeded".into())
}

#[allow(clippy::too_many_arguments)]
async fn ingest_text(
    vendor: &AlpacaMarkets,
    master: &InstrumentMaster,
    supervisor: &mut FeedSupervisor,
    hub: &Arc<Mutex<FanoutHub<TokenAuth>>>,
    history: &Arc<Mutex<HistoricalArchive>>,
    http: &reqwest::Client,
    ws: &mut WsStream,
    now: u64,
    text: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let frames: Vec<Vec<u8>> = match serde_json::from_str::<Value>(text) {
        Ok(Value::Array(items)) => items
            .into_iter()
            .map(|v| v.to_string().into_bytes())
            .collect(),
        _ => vec![text.as_bytes().to_vec()],
    };
    for raw in frames {
        let outcome = supervisor.ingest(vendor, master, now, &raw);
        dispatch_outcome(hub, history, &outcome);
        execute_commands(
            ws,
            http,
            vendor,
            master,
            supervisor,
            hub,
            history,
            outcome.commands,
        )
        .await?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn execute_commands(
    ws: &mut WsStream,
    http: &reqwest::Client,
    vendor: &AlpacaMarkets,
    master: &InstrumentMaster,
    supervisor: &mut FeedSupervisor,
    hub: &Arc<Mutex<FanoutHub<TokenAuth>>>,
    history: &Arc<Mutex<HistoricalArchive>>,
    commands: Vec<FeedCommand>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    for cmd in commands {
        match cmd {
            FeedCommand::Subscribe { payload } => {
                let text = String::from_utf8_lossy(&payload).into_owned();
                ws.send(Message::Text(text.into())).await?;
            }
            FeedCommand::RequestSnapshot {
                product_id, spec, ..
            } => {
                let mut req = http.get(spec.url());
                for (k, v) in spec.headers() {
                    req = req.header(k.as_str(), v.as_str());
                }
                match req.send().await {
                    Ok(resp) => {
                        let body = resp.bytes().await?;
                        let now = unix_logical_now();
                        match supervisor.ingest_snapshot(vendor, master, now, &product_id, &body) {
                            Ok(outcome) => {
                                dispatch_outcome(hub, history, &outcome);
                                Box::pin(execute_commands(
                                    ws,
                                    http,
                                    vendor,
                                    master,
                                    supervisor,
                                    hub,
                                    history,
                                    outcome.commands,
                                ))
                                .await?;
                            }
                            Err(err) => {
                                eprintln!(
                                    "shinrai-md-gateway: alpaca snapshot failed for {product_id}: {err}"
                                );
                            }
                        }
                    }
                    Err(err) => {
                        eprintln!(
                            "shinrai-md-gateway: alpaca snapshot HTTP failed for {product_id}: {err}"
                        );
                    }
                }
            }
            FeedCommand::Reconnect { delay_logical, .. } => {
                tokio::time::sleep(Duration::from_secs(delay_logical.max(1))).await;
                return Err("reconnect requested".into());
            }
        }
    }
    Ok(())
}

fn dispatch_outcome(
    hub: &Arc<Mutex<FanoutHub<TokenAuth>>>,
    history: &Arc<Mutex<HistoricalArchive>>,
    outcome: &IngestOutcome,
) {
    for event in &outcome.events {
        dispatch_event(hub, history, event);
    }
}
