//! Live Coinbase Exchange feed I/O (sockets stay in the gateway).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use shinrai_instruments::{btc_usd, InstrumentMaster};
use shinrai_market_data::{HistoricalArchive, MdKind};
use shinrai_md_fanout::{FanoutHub, TokenAuth};
use shinrai_md_protocol::{
    CoinbaseExchange, FeedCommand, FeedSupervisor, IngestOutcome, MarketDataVendor, SupervisorEvent,
};
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::app::unix_logical_now;
use crate::map::to_market_event;

/// Default silence SLA (unix seconds) before [`SupervisorEvent::Stale`].
const SLA_SECS: u64 = 30;

type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Spawns a background task that connects to Coinbase public WS + REST snapshots.
pub(crate) fn spawn(
    hub: Arc<Mutex<FanoutHub<TokenAuth>>>,
    history: Arc<Mutex<HistoricalArchive>>,
    master: InstrumentMaster,
) {
    tokio::spawn(async move {
        if let Err(err) = run_loop(hub, history, master).await {
            eprintln!("shinrai-md-gateway: coinbase feed stopped: {err}");
        }
    });
}

async fn run_loop(
    hub: Arc<Mutex<FanoutHub<TokenAuth>>>,
    history: Arc<Mutex<HistoricalArchive>>,
    master: InstrumentMaster,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let vendor = CoinbaseExchange;
    let http = reqwest::Client::builder()
        .user_agent("shinrai-md-gateway/0.1")
        .timeout(Duration::from_secs(10))
        .build()?;

    let mut delay = Duration::from_secs(1);
    loop {
        match session(&vendor, &http, &hub, &history, &master).await {
            Ok(()) => {
                eprintln!("shinrai-md-gateway: coinbase session ended; reconnecting");
                delay = Duration::from_secs(1);
            }
            Err(err) => {
                eprintln!("shinrai-md-gateway: coinbase session error: {err}; retry in {delay:?}");
            }
        }
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(Duration::from_secs(32));
    }
}

async fn session(
    vendor: &CoinbaseExchange,
    http: &reqwest::Client,
    hub: &Arc<Mutex<FanoutHub<TokenAuth>>>,
    history: &Arc<Mutex<HistoricalArchive>>,
    master: &InstrumentMaster,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut supervisor = FeedSupervisor::new(SLA_SECS);
    supervisor.watch(btc_usd().id(), "BTC-USD");

    let url = vendor.websocket_url();
    eprintln!("shinrai-md-gateway: coinbase connecting → {url}");
    let (mut ws, _) = connect_async(url).await?;

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
                        let outcome = supervisor.ingest(vendor, master, now, text.as_bytes());
                        dispatch_outcome(hub, history, &outcome);
                        execute_commands(
                            &mut ws,
                            http,
                            vendor,
                            master,
                            &mut supervisor,
                            hub,
                            history,
                            outcome.commands,
                        )
                        .await?;
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        ws.send(Message::Pong(payload)).await?;
                    }
                    None | Some(Ok(Message::Close(_)) | Err(_)) => {
                        handle_disconnect(
                            &mut ws, http, vendor, master, &mut supervisor, hub, history,
                        )
                        .await?;
                        return Ok(());
                    }
                    Some(Ok(_)) => {}
                }
            }
        }
    }
}

async fn handle_disconnect(
    ws: &mut WsStream,
    http: &reqwest::Client,
    vendor: &CoinbaseExchange,
    master: &InstrumentMaster,
    supervisor: &mut FeedSupervisor,
    hub: &Arc<Mutex<FanoutHub<TokenAuth>>>,
    history: &Arc<Mutex<HistoricalArchive>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let commands = supervisor.on_disconnect();
    dispatch_event(
        hub,
        history,
        &SupervisorEvent::Stale {
            instrument_id: btc_usd().id(),
            silent_for: SLA_SECS.saturating_add(1),
        },
    );
    let _ = execute_commands(ws, http, vendor, master, supervisor, hub, history, commands).await;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn execute_commands(
    ws: &mut WsStream,
    http: &reqwest::Client,
    vendor: &CoinbaseExchange,
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
                let body = http.get(spec.url()).send().await?.bytes().await?;
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
                            "shinrai-md-gateway: coinbase snapshot failed for {product_id}: {err}"
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

/// Applies one supervisor event to hub + archive (testable without sockets).
pub(crate) fn dispatch_event(
    hub: &Arc<Mutex<FanoutHub<TokenAuth>>>,
    history: &Arc<Mutex<HistoricalArchive>>,
    event: &SupervisorEvent,
) {
    if let SupervisorEvent::Applied(record) = event {
        if matches!(
            record.kind(),
            MdKind::Trade | MdKind::Bbo | MdKind::Snapshot
        ) {
            let mut archive = history
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let _ = archive.ingest(*record);
        }
    }
    if let Some(ev) = to_market_event(event) {
        hub.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .publish(ev);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shinrai_instruments::phase1_master;
    use shinrai_md_fanout::{FanoutConfig, TokenTtl};

    #[test]
    fn fixture_snapshot_then_ticker_fans_out() {
        let vendor = CoinbaseExchange;
        let master = phase1_master();
        let mut supervisor = FeedSupervisor::new(30);
        supervisor.watch(btc_usd().id(), "BTC-USD");
        let _ = supervisor.on_connected(&vendor, 1);

        let snap =
            include_bytes!("../../../protocols/market-data/tests/fixtures/book_snapshot.json");
        let ticker = include_bytes!("../../../protocols/market-data/tests/fixtures/ticker.json");

        let snap_out = supervisor
            .ingest_snapshot(&vendor, &master, 1, "BTC-USD", snap)
            .expect("snap");
        assert!(
            snap_out.events.iter().any(|e| matches!(
                e,
                SupervisorEvent::SnapshotRecovered { .. } | SupervisorEvent::Applied(_)
            )),
            "expected snapshot recovery: {:?}",
            snap_out.events
        );

        let tick = supervisor.ingest(&vendor, &master, 2, ticker);
        assert!(
            tick.events
                .iter()
                .any(|e| matches!(e, SupervisorEvent::Applied(_))),
            "expected applied ticker: {:?}",
            tick.events
        );

        let auth = TokenAuth::new(TokenTtl::default());
        let hub = Arc::new(Mutex::new(FanoutHub::new(
            FanoutConfig::default(),
            auth,
            master.clone(),
        )));
        let history = Arc::new(Mutex::new(HistoricalArchive::default_intervals()));
        for event in tick.events.iter().chain(snap_out.events.iter()) {
            dispatch_event(&hub, &history, event);
        }
        assert!(
            !history.lock().expect("h").journal().is_empty(),
            "archive should receive applied records"
        );
    }
}
