//! Axum router, session loop, and env configuration.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use shinrai_instruments::{btc_usd, phase1_master, InstrumentMaster};
use shinrai_market_data::{HistoricalArchive, MdJournal, MdKind, MdRecord, SyntheticFeed};
use shinrai_md_fanout::{
    encode_message, ClientMessage, FanoutConfig, FanoutError, FanoutHub, MarketEvent,
    StaticTokenAuth, SubjectId,
};

/// Shared gateway state (mutex only at this I/O edge).
#[derive(Clone)]
pub struct AppState {
    pub(crate) hub: Arc<Mutex<FanoutHub<StaticTokenAuth>>>,
    pub(crate) history: Arc<Mutex<HistoricalArchive>>,
    pub(crate) master: InstrumentMaster,
}

/// Process configuration (tokens are not displayed).
#[derive(Clone)]
pub struct GatewayConfig {
    tokens: Vec<(String, String)>,
    fanout: FanoutConfig,
    synth: bool,
}

impl GatewayConfig {
    /// Builds a config. Empty `tokens` is fail-closed (every connect rejected).
    #[must_use]
    pub fn new(tokens: Vec<(String, String)>, fanout: FanoutConfig, synth: bool) -> Self {
        Self {
            tokens,
            fanout,
            synth,
        }
    }

    /// Reads `SHINRAI_MD_TOKENS` (`token:subject,...`) and `SHINRAI_MD_SYNTH`.
    #[must_use]
    pub fn from_env() -> Self {
        Self::new(
            parse_tokens(std::env::var("SHINRAI_MD_TOKENS").ok().as_deref()),
            FanoutConfig::default(),
            env_flag("SHINRAI_MD_SYNTH"),
        )
    }

    /// Whether a synthetic publisher should run.
    #[must_use]
    pub const fn synth(&self) -> bool {
        self.synth
    }
}

impl core::fmt::Debug for GatewayConfig {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GatewayConfig")
            .field("token_entries", &self.tokens.len())
            .field("fanout", &self.fanout)
            .field("synth", &self.synth)
            .finish()
    }
}

impl AppState {
    /// Builds state from config (does not spawn the synth task).
    #[must_use]
    pub fn from_config(config: &GatewayConfig) -> Self {
        let auth = StaticTokenAuth::new();
        for (token, subject) in &config.tokens {
            auth.grant(token, SubjectId::new(subject.clone()));
        }
        let master = phase1_master();
        let mut history = HistoricalArchive::default_intervals();
        if let Ok(seed) = demo_seed_journal() {
            let _ = history.load_journal(&seed);
        }
        Self {
            hub: Arc::new(Mutex::new(FanoutHub::new(
                config.fanout,
                auth,
                master.clone(),
            ))),
            history: Arc::new(Mutex::new(history)),
            master,
        }
    }

    /// Test helper with a single granted token.
    #[must_use]
    pub fn for_test(token: &str, subject: &str) -> Self {
        Self::from_config(&GatewayConfig::new(
            vec![(token.to_owned(), subject.to_owned())],
            FanoutConfig::default(),
            false,
        ))
    }

    /// Publishes synthetic BTC-USD trades (local demo only).
    pub fn spawn_synth(&self) {
        let hub = Arc::clone(&self.hub);
        let history = Arc::clone(&self.history);
        tokio::spawn(async move {
            let mut feed = SyntheticFeed::new(1, btc_usd().id(), 6_500_000);
            let mut interval = tokio::time::interval(Duration::from_millis(100));
            loop {
                interval.tick().await;
                let record = feed.next_trade();
                let ts = unix_logical_now();
                let record = MdRecord::new(
                    record.instrument_id(),
                    record.seq(),
                    ts,
                    MdKind::Trade,
                    record.price(),
                )
                .with_qty(record.qty());
                {
                    let mut archive = history
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    let _ = archive.ingest(record);
                }
                hub.lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .publish(MarketEvent::Tick(record));
            }
        });
    }
}

/// Logical clock used by the hub (unix seconds).
#[must_use]
pub fn unix_logical_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// HTTP router: health, historical REST, WebSocket.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v1/bars", get(crate::historical::get_bars))
        .route("/v1/trades", get(crate::historical::get_trades))
        .route("/v1/ws", get(ws_handler))
        .with_state(state)
}

async fn health() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

/// Token query param shared by WebSocket and REST.
#[derive(Debug, Deserialize)]
pub struct AuthQuery {
    pub token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WsQuery {
    token: Option<String>,
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    Query(query): Query<WsQuery>,
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Response {
    let token = extract_bearer(&headers, &AuthQuery { token: query.token });
    let preview = lock_hub(&state).preview_auth(token.as_deref());
    match preview {
        Ok(()) => ws.on_upgrade(move |socket| client_session(socket, state, token)),
        Err(err) => unauthorized(err),
    }
}

pub(crate) fn unauthorized(err: FanoutError) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({ "type": "error", "code": err.code() })),
    )
        .into_response()
}

pub(crate) fn extract_bearer(headers: &HeaderMap, query: &AuthQuery) -> Option<String> {
    if let Some(value) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    {
        if let Some((scheme, rest)) = value.split_once(' ') {
            if scheme.eq_ignore_ascii_case("bearer") {
                let rest = rest.trim();
                if !rest.is_empty() {
                    return Some(rest.to_owned());
                }
            }
        }
    }
    query
        .token
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

async fn client_session(mut socket: WebSocket, state: AppState, token: Option<String>) {
    let now = unix_logical_now();
    let connected = {
        let mut hub = lock_hub(&state);
        hub.connect(token.as_deref(), now)
    };
    let session_id = match connected {
        Ok(id) => id,
        Err(err) => {
            let _ = send_msg(&mut socket, error_frame(err)).await;
            return;
        }
    };
    let mut tick = tokio::time::interval(Duration::from_millis(50));
    loop {
        tokio::select! {
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        let now = unix_logical_now();
                        let msgs = {
                            let mut hub = lock_hub(&state);
                            let _ = hub.handle_text(session_id, text.as_bytes(), now);
                            hub.drain(session_id)
                        };
                        if flush(&mut socket, &msgs).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        if socket.send(Message::Pong(payload)).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Close(_)) | Err(_)) | None => break,
                    Some(Ok(Message::Pong(_) | Message::Binary(_))) => {}
                }
            }
            _ = tick.tick() => {
                let now = unix_logical_now();
                let (msgs, closed) = {
                    let mut hub = lock_hub(&state);
                    let _ = hub.on_clock(now);
                    let open = hub.is_open(session_id);
                    let msgs = hub.drain(session_id);
                    if !open {
                        hub.disconnect(session_id);
                    }
                    (msgs, !open)
                };
                if flush(&mut socket, &msgs).await.is_err() {
                    break;
                }
                if closed {
                    break;
                }
            }
        }
    }
    lock_hub(&state).disconnect(session_id);
}

async fn flush(socket: &mut WebSocket, msgs: &[ClientMessage]) -> Result<(), ()> {
    for msg in msgs {
        send_msg(socket, encode_message(msg)).await?;
    }
    Ok(())
}

async fn send_msg(socket: &mut WebSocket, bytes: Vec<u8>) -> Result<(), ()> {
    let text = String::from_utf8(bytes).map_err(|_| ())?;
    socket.send(Message::text(text)).await.map_err(|_| ())
}

fn error_frame(err: FanoutError) -> Vec<u8> {
    encode_message(&ClientMessage::Error { code: err.code() })
}

fn parse_tokens(raw: Option<&str>) -> Vec<(String, String)> {
    let Some(raw) = raw.filter(|s| !s.trim().is_empty()) else {
        return Vec::new();
    };
    raw.split(',')
        .filter_map(|part| {
            let (token, subject) = part.split_once(':')?;
            let token = token.trim();
            let subject = subject.trim();
            if token.is_empty() || subject.is_empty() {
                return None;
            }
            Some((token.to_owned(), subject.to_owned()))
        })
        .collect()
}

fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .is_some_and(|v| matches!(v.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
}

pub(crate) fn lock_hub(state: &AppState) -> MutexGuard<'_, FanoutHub<StaticTokenAuth>> {
    state
        .hub
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub(crate) fn lock_archive(state: &AppState) -> MutexGuard<'_, HistoricalArchive> {
    state
        .history
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Deterministic seed trades for historical API smoke tests.
fn demo_seed_journal() -> Result<MdJournal, shinrai_market_data::MdError> {
    SyntheticFeed::record_trades(42, btc_usd().id(), 120)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tokens_skips_empty() {
        assert!(parse_tokens(None).is_empty());
        assert_eq!(
            parse_tokens(Some("a:alice, b:bob")),
            vec![("a".into(), "alice".into()), ("b".into(), "bob".into())]
        );
    }

    #[test]
    fn debug_config_hides_tokens() {
        let cfg = GatewayConfig::new(
            vec![("super-secret-token".into(), "alice".into())],
            FanoutConfig::default(),
            false,
        );
        let rendered = format!("{cfg:?}");
        assert!(!rendered.contains("super-secret-token"));
    }

    #[test]
    fn demo_seed_loads_bars() {
        let state = AppState::for_test("t", "alice");
        let archive = lock_archive(&state);
        assert!(!archive.journal().is_empty());
        assert!(!archive.bars().store().is_empty());
    }
}
