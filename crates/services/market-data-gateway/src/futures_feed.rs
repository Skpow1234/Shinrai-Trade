//! CME Globex futures MD I/O (Databento-shaped JSON) — paper or licensed live.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use shinrai_instruments::{esz5, ExternalId, InstrumentMaster};
use shinrai_market_data::HistoricalArchive;
use shinrai_md_fanout::{FanoutHub, TokenAuth};
use shinrai_md_protocol::{
    DatabentoGlbx, FeedCommand, FeedSupervisor, IngestOutcome, SupervisorEvent,
};
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::app::unix_logical_now;
use crate::coinbase_feed::dispatch_event;

const SLA_SECS: u64 = 30;
/// ESZ5 paper start — $5200.00 on the 0.25 tick grid (scaled ticks).
const PAPER_START_SCALED: i64 = 520_000;

type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Spawns futures MD for the given CME product ids (paper unless live endpoints+key).
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn(
    hub: Arc<Mutex<FanoutHub<TokenAuth>>>,
    history: Arc<Mutex<HistoricalArchive>>,
    master: InstrumentMaster,
    products: Vec<String>,
    api_key: Option<String>,
    ws_url: Option<String>,
    rest_url: Option<String>,
    dataset: Option<String>,
    schema: Option<String>,
) {
    let live = api_key.as_ref().is_some_and(|k| !k.is_empty())
        && ws_url
            .as_ref()
            .is_some_and(|u| !u.is_empty() && !u.starts_with("paper://"));
    tokio::spawn(async move {
        if live {
            let key = api_key.unwrap_or_default();
            if let Err(err) = run_live(
                hub,
                history,
                master,
                products,
                key,
                ws_url.unwrap_or_else(|| DatabentoGlbx::WS_DEFAULT.into()),
                rest_url.unwrap_or_else(|| DatabentoGlbx::REST_DEFAULT.into()),
                dataset.unwrap_or_else(|| DatabentoGlbx::DATASET_GLBX.into()),
                schema.unwrap_or_else(|| DatabentoGlbx::SCHEMA_MBP10.into()),
            )
            .await
            {
                eprintln!("shinrai-md-gateway: futures live feed stopped: {err}");
            }
        } else {
            eprintln!(
                "shinrai-md-gateway: futures paper feed (Databento GLBX mbp-10) products={products:?}"
            );
            if let Err(err) = run_paper(hub, history, master, products).await {
                eprintln!("shinrai-md-gateway: futures paper feed stopped: {err}");
            }
        }
    });
}

async fn run_paper(
    hub: Arc<Mutex<FanoutHub<TokenAuth>>>,
    history: Arc<Mutex<HistoricalArchive>>,
    master: InstrumentMaster,
    products: Vec<String>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let vendor = DatabentoGlbx::paper();
    let mut supervisor = FeedSupervisor::new(SLA_SECS);
    let products = if products.is_empty() {
        vec!["ESZ5".into()]
    } else {
        products
    };
    for product in &products {
        let alias = ExternalId::ticker(product)?;
        let id = master.resolve_alias(&alias)?;
        supervisor.watch(id, product.clone());
    }

    let now = unix_logical_now();
    let commands = supervisor.on_connected(&vendor, now);
    apply_paper_commands(
        &vendor,
        &master,
        &mut supervisor,
        &hub,
        &history,
        commands,
        PAPER_START_SCALED,
    )?;

    let mut scaled = PAPER_START_SCALED;
    let mut seq = 100_u64;
    let mut turn = 0_usize;
    let mut interval = tokio::time::interval(Duration::from_millis(100));
    let mut clock = tokio::time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            _ = clock.tick() => {
                let now = unix_logical_now();
                for event in supervisor.on_clock(now) {
                    dispatch_event(&hub, &history, &event);
                }
            }
            _ = interval.tick() => {
                let product = &products[turn % products.len()];
                turn = turn.saturating_add(1);
                // Walk the 0.25 tick grid (±1 tick).
                let step = if seq.is_multiple_of(2) { 25 } else { -25 };
                scaled = (scaled + step).max(100_000);
                seq = seq.saturating_add(1);
                let price = format_esz_price(scaled);
                let now = unix_logical_now();
                // Alternate trade prints and mbp-10 L2 snapshots (fuller Databento wire).
                let raw = if seq.is_multiple_of(3) {
                    DatabentoGlbx::paper_book_body(product, seq, &price)
                } else {
                    DatabentoGlbx::paper_trade_body(product, seq, &price, 1)
                };
                let outcome = supervisor.ingest(&vendor, &master, now, &raw);
                dispatch_outcome(&hub, &history, &outcome);
                apply_paper_commands(
                    &vendor,
                    &master,
                    &mut supervisor,
                    &hub,
                    &history,
                    outcome.commands,
                    scaled,
                )?;
            }
        }
    }
}

fn apply_paper_commands(
    vendor: &DatabentoGlbx,
    master: &InstrumentMaster,
    supervisor: &mut FeedSupervisor,
    hub: &Arc<Mutex<FanoutHub<TokenAuth>>>,
    history: &Arc<Mutex<HistoricalArchive>>,
    commands: Vec<FeedCommand>,
    mark_scaled: i64,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    for cmd in commands {
        match cmd {
            FeedCommand::Subscribe { .. } => {}
            FeedCommand::RequestSnapshot { product_id, .. } => {
                let price = format_esz_price(mark_scaled);
                let body = DatabentoGlbx::paper_snapshot_body(&product_id, 1, &price);
                let now = unix_logical_now();
                match supervisor.ingest_snapshot(vendor, master, now, &product_id, &body) {
                    Ok(outcome) => {
                        dispatch_outcome(hub, history, &outcome);
                        apply_paper_commands(
                            vendor,
                            master,
                            supervisor,
                            hub,
                            history,
                            outcome.commands,
                            mark_scaled,
                        )?;
                    }
                    Err(err) => {
                        eprintln!(
                            "shinrai-md-gateway: futures paper snapshot failed for {product_id}: {err}"
                        );
                    }
                }
            }
            FeedCommand::Reconnect { delay_logical, .. } => {
                return Err(format!("paper reconnect requested delay={delay_logical}").into());
            }
        }
    }
    Ok(())
}

fn format_esz_price(scaled: i64) -> String {
    // price_scale=2 → dollars with 2dp (tick 0.25 still validates on ingest).
    let whole = scaled / 100;
    let frac = (scaled % 100).unsigned_abs();
    format!("{whole}.{frac:02}")
}

#[allow(clippy::too_many_arguments)]
async fn run_live(
    hub: Arc<Mutex<FanoutHub<TokenAuth>>>,
    history: Arc<Mutex<HistoricalArchive>>,
    master: InstrumentMaster,
    products: Vec<String>,
    api_key: String,
    ws_url: String,
    rest_url: String,
    dataset: String,
    schema: String,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let vendor = DatabentoGlbx::live(api_key)
        .with_endpoints(ws_url, rest_url)
        .with_dataset(dataset)
        .with_schema(schema);
    let http = reqwest::Client::builder()
        .user_agent("shinrai-md-gateway/0.1")
        .timeout(Duration::from_secs(10))
        .build()?;
    let mut delay = Duration::from_secs(1);
    loop {
        match live_session(&vendor, &http, &hub, &history, &master, &products).await {
            Ok(()) => {
                eprintln!("shinrai-md-gateway: futures live session ended; reconnecting");
                delay = Duration::from_secs(1);
            }
            Err(err) => {
                eprintln!("shinrai-md-gateway: futures live error: {err}; retry in {delay:?}");
            }
        }
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(Duration::from_secs(32));
    }
}

async fn live_session(
    vendor: &DatabentoGlbx,
    http: &reqwest::Client,
    hub: &Arc<Mutex<FanoutHub<TokenAuth>>>,
    history: &Arc<Mutex<HistoricalArchive>>,
    master: &InstrumentMaster,
    products: &[String],
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut supervisor = FeedSupervisor::new(SLA_SECS);
    let products = if products.is_empty() {
        vec!["ESZ5".into()]
    } else {
        products.to_vec()
    };
    for product in &products {
        let alias = ExternalId::ticker(product)?;
        let id = master.resolve_alias(&alias)?;
        supervisor.watch(id, product.clone());
    }

    let url = vendor.stream_url();
    eprintln!("shinrai-md-gateway: futures live connecting → {url}");
    let (mut ws, _) = connect_async(url).await?;
    let auth = String::from_utf8(vendor.auth_message())?;
    ws.send(Message::Text(auth.into())).await?;

    let now = unix_logical_now();
    let commands = supervisor.on_connected(vendor, now);
    execute_live_commands(
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
                        execute_live_commands(
                            &mut ws, http, vendor, master, &mut supervisor, hub, history, outcome.commands,
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
                                instrument_id: esz5().id(),
                                silent_for: SLA_SECS.saturating_add(1),
                            },
                        );
                        let _ = execute_live_commands(
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

#[allow(clippy::too_many_arguments)]
async fn execute_live_commands(
    ws: &mut WsStream,
    http: &reqwest::Client,
    vendor: &DatabentoGlbx,
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
                if spec.url().starts_with("paper://") {
                    let body = DatabentoGlbx::paper_snapshot_body(&product_id, 1, "5200.00");
                    let now = unix_logical_now();
                    if let Ok(outcome) =
                        supervisor.ingest_snapshot(vendor, master, now, &product_id, &body)
                    {
                        dispatch_outcome(hub, history, &outcome);
                    }
                    continue;
                }
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
                                Box::pin(execute_live_commands(
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
                                    "shinrai-md-gateway: futures snapshot failed for {product_id}: {err}"
                                );
                            }
                        }
                    }
                    Err(err) => {
                        eprintln!(
                            "shinrai-md-gateway: futures snapshot HTTP failed for {product_id}: {err}"
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

#[cfg(test)]
mod tests {
    use super::*;
    use shinrai_instruments::phase1_master;
    use shinrai_md_fanout::{FanoutConfig, TokenAuth, TokenTtl};
    use shinrai_md_protocol::SupervisorEvent;

    #[test]
    fn paper_snapshot_then_trade_via_supervisor() {
        let vendor = DatabentoGlbx::paper();
        let master = phase1_master();
        let mut supervisor = FeedSupervisor::new(30);
        supervisor.watch(esz5().id(), "ESZ5");
        let cmds = supervisor.on_connected(&vendor, 1);
        assert!(cmds
            .iter()
            .any(|c| matches!(c, FeedCommand::RequestSnapshot { .. })));
        let body = DatabentoGlbx::paper_snapshot_body("ESZ5", 10, "5200.00");
        let snap = supervisor
            .ingest_snapshot(&vendor, &master, 1, "ESZ5", &body)
            .expect("snap");
        assert!(matches!(
            snap.events[0],
            SupervisorEvent::SnapshotRecovered { .. }
        ));
        let trade = DatabentoGlbx::paper_trade_body("ESZ5", 11, "5200.25", 1);
        let out = supervisor.ingest(&vendor, &master, 2, &trade);
        assert!(matches!(out.events[0], SupervisorEvent::Applied(_)));
        let _hub = FanoutHub::new(
            FanoutConfig::default(),
            TokenAuth::new(TokenTtl::default()),
            master,
        );
    }

    #[test]
    fn format_esz_price_two_dp() {
        assert_eq!(format_esz_price(520_000), "5200.00");
        assert_eq!(format_esz_price(520_025), "5200.25");
    }
}
