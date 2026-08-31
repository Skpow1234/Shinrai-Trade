//! Axum router, session loop, and env configuration.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use shinrai_instruments::{btc_usd, phase1_master, InstrumentMaster};
use shinrai_market_data::{HistoricalArchive, MdJournal, MdKind, MdRecord, SyntheticFeed};
use shinrai_md_fanout::{
    encode_message, ClientMessage, FanoutConfig, FanoutError, FanoutHub, MarketEvent, SubjectId,
    TokenAuth, TokenTtl,
};

/// Shared gateway state (mutex only at this I/O edge).
#[derive(Clone)]
pub struct AppState {
    pub(crate) hub: Arc<Mutex<FanoutHub<TokenAuth>>>,
    pub(crate) auth: TokenAuth,
    pub(crate) history: Arc<Mutex<HistoricalArchive>>,
    pub(crate) master: InstrumentMaster,
}

/// Process configuration (tokens / secrets are not displayed).
#[derive(Clone)]
pub struct GatewayConfig {
    static_tokens: Vec<(String, String)>,
    clients: Vec<(String, String, String)>,
    fanout: FanoutConfig,
    ttl: TokenTtl,
    synth: bool,
}

impl GatewayConfig {
    /// Builds a config. Empty clients and static tokens is fail-closed.
    #[must_use]
    pub fn new(
        static_tokens: Vec<(String, String)>,
        clients: Vec<(String, String, String)>,
        fanout: FanoutConfig,
        ttl: TokenTtl,
        synth: bool,
    ) -> Self {
        Self {
            static_tokens,
            clients,
            fanout,
            ttl,
            synth,
        }
    }

    /// Reads env: `SHINRAI_MD_TOKENS`, `SHINRAI_MD_CLIENTS`, TTLs, synth.
    #[must_use]
    pub fn from_env() -> Self {
        let access = env_u64("SHINRAI_MD_ACCESS_TTL").unwrap_or(60);
        let refresh = env_u64("SHINRAI_MD_REFRESH_TTL").unwrap_or(3_600);
        Self::new(
            parse_tokens(std::env::var("SHINRAI_MD_TOKENS").ok().as_deref()),
            parse_clients(std::env::var("SHINRAI_MD_CLIENTS").ok().as_deref()),
            FanoutConfig::default(),
            TokenTtl::new(access, refresh),
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
            .field("static_token_entries", &self.static_tokens.len())
            .field("client_entries", &self.clients.len())
            .field("fanout", &self.fanout)
            .field("ttl", &self.ttl)
            .field("synth", &self.synth)
            .finish()
    }
}

impl AppState {
    /// Builds state from config (does not spawn the synth task).
    #[must_use]
    pub fn from_config(config: &GatewayConfig) -> Self {
        let auth = TokenAuth::new(config.ttl);
        for (token, subject) in &config.static_tokens {
            auth.grant_static_access(token, SubjectId::new(subject.clone()));
        }
        for (id, secret, subject) in &config.clients {
            auth.register_client(id, secret, SubjectId::new(subject.clone()));
        }
        let master = phase1_master();
        let mut history = HistoricalArchive::default_intervals();
        if let Ok(seed) = demo_seed_journal() {
            let _ = history.load_journal(&seed);
        }
        Self {
            hub: Arc::new(Mutex::new(FanoutHub::new(
                config.fanout,
                auth.clone(),
                master.clone(),
            ))),
            auth,
            history: Arc::new(Mutex::new(history)),
            master,
        }
    }

    /// Test helper with a single static access token.
    #[must_use]
    pub fn for_test(token: &str, subject: &str) -> Self {
        Self::from_config(&GatewayConfig::new(
            vec![(token.to_owned(), subject.to_owned())],
            Vec::new(),
            FanoutConfig::default(),
            TokenTtl::default(),
            false,
        ))
    }

    /// Test helper with a client credential pair.
    #[must_use]
    pub fn for_test_client(client_id: &str, secret: &str, subject: &str) -> Self {
        Self::from_config(&GatewayConfig::new(
            Vec::new(),
            vec![(client_id.to_owned(), secret.to_owned(), subject.to_owned())],
            FanoutConfig::default(),
            TokenTtl::new(60, 3_600),
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

/// HTTP router: health, auth, historical REST, WebSocket.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v1/auth/token", post(crate::auth_http::post_token))
        .route("/v1/auth/revoke", post(crate::auth_http::post_revoke))
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
    let now = unix_logical_now();
    let preview = lock_hub(&state).preview_auth(token.as_deref(), now);
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

fn parse_clients(raw: Option<&str>) -> Vec<(String, String, String)> {
    let Some(raw) = raw.filter(|s| !s.trim().is_empty()) else {
        return Vec::new();
    };
    raw.split(',')
        .filter_map(|part| {
            let mut bits = part.splitn(3, ':');
            let id = bits.next()?.trim();
            let secret = bits.next()?.trim();
            let subject = bits.next()?.trim();
            if id.is_empty() || secret.is_empty() || subject.is_empty() {
                return None;
            }
            Some((id.to_owned(), secret.to_owned(), subject.to_owned()))
        })
        .collect()
}

fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .is_some_and(|v| matches!(v.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
}

fn env_u64(name: &str) -> Option<u64> {
    std::env::var(name).ok()?.parse().ok()
}

pub(crate) fn lock_hub(state: &AppState) -> MutexGuard<'_, FanoutHub<TokenAuth>> {
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
    fn parse_clients_three_fields() {
        assert_eq!(
            parse_clients(Some("dev:s3cret:alice")),
            vec![("dev".into(), "s3cret".into(), "alice".into())]
        );
    }

    #[test]
    fn debug_config_hides_secrets() {
        let cfg = GatewayConfig::new(
            vec![("super-secret-token".into(), "alice".into())],
            vec![("cli".into(), "super-secret".into(), "alice".into())],
            FanoutConfig::default(),
            TokenTtl::default(),
            false,
        );
        let rendered = format!("{cfg:?}");
        assert!(!rendered.contains("super-secret-token"));
        assert!(!rendered.contains("super-secret"));
    }

    #[test]
    fn demo_seed_loads_bars() {
        let state = AppState::for_test("t", "alice");
        let archive = lock_archive(&state);
        assert!(!archive.journal().is_empty());
        assert!(!archive.bars().store().is_empty());
    }
}
