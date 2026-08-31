//! Session hub: subscriptions, bounded outbound queues, heartbeats, TTL.

use std::collections::{HashMap, HashSet, VecDeque};

use shinrai_instruments::{ExternalId, IdType, InstrumentId, InstrumentMaster};

use crate::auth::Authenticator;
use crate::error::FanoutError;
use crate::protocol::ClientCommand;
use crate::session::{ClientMessage, MarketEvent, SessionId};

/// Hub timing and queue limits (logical clock units).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FanoutConfig {
    queue_capacity: usize,
    max_subscriptions: usize,
    heartbeat_every_logical: u64,
    session_ttl_logical: u64,
}

impl FanoutConfig {
    /// Builds a config, clamping empty/zero fields to 1.
    #[must_use]
    pub fn new(
        queue_capacity: usize,
        max_subscriptions: usize,
        heartbeat_every_logical: u64,
        session_ttl_logical: u64,
    ) -> Self {
        Self {
            queue_capacity: queue_capacity.max(1),
            max_subscriptions: max_subscriptions.max(1),
            heartbeat_every_logical: heartbeat_every_logical.max(1),
            session_ttl_logical: session_ttl_logical.max(1),
        }
    }

    /// Outbound queue length (oldest market-data is dropped on overflow).
    #[must_use]
    pub const fn queue_capacity(self) -> usize {
        self.queue_capacity
    }

    /// Max distinct instruments per session.
    #[must_use]
    pub const fn max_subscriptions(self) -> usize {
        self.max_subscriptions
    }

    /// Server heartbeat period in logical time.
    #[must_use]
    pub const fn heartbeat_every_logical(self) -> u64 {
        self.heartbeat_every_logical
    }

    /// Idle session lifetime in logical time.
    #[must_use]
    pub const fn session_ttl_logical(self) -> u64 {
        self.session_ttl_logical
    }
}

impl Default for FanoutConfig {
    fn default() -> Self {
        Self::new(64, 16, 15, 45)
    }
}

/// Why a session was closed by [`FanoutHub::on_clock`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseReason {
    /// No client activity within the TTL.
    Ttl,
    /// Access token revoked (or no longer granted).
    Revoked,
}

/// Sessions closed on a clock tick (still drainable until [`FanoutHub::disconnect`]).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ClockOutcome {
    /// Sessions marked dead this tick.
    pub closed: Vec<(SessionId, CloseReason)>,
}

struct Session {
    token: String,
    subscriptions: HashSet<InstrumentId>,
    outbound: VecDeque<ClientMessage>,
    dropped: u64,
    last_activity_logical: u64,
    last_heartbeat_logical: u64,
    dead: bool,
}

impl Session {
    fn enqueue(&mut self, capacity: usize, msg: ClientMessage) {
        if self.outbound.len() >= capacity {
            self.outbound.pop_front();
            self.dropped = self.dropped.saturating_add(1);
        }
        self.outbound.push_back(msg);
    }
}

/// Transport-agnostic fanout (single-threaded; wrap in a mutex at the I/O edge).
pub struct FanoutHub<A> {
    config: FanoutConfig,
    auth: A,
    master: InstrumentMaster,
    sessions: HashMap<SessionId, Session>,
    next_id: u64,
}

impl<A: Authenticator> FanoutHub<A> {
    /// Creates an empty hub.
    #[must_use]
    pub fn new(config: FanoutConfig, auth: A, master: InstrumentMaster) -> Self {
        Self {
            config,
            auth,
            master,
            sessions: HashMap::new(),
            next_id: 1,
        }
    }

    /// Validates a token without opening a session (HTTP 401 gate).
    ///
    /// # Errors
    ///
    /// Missing, invalid, revoked, or expired tokens.
    pub fn preview_auth(&self, token: Option<&str>, now_logical: u64) -> Result<(), FanoutError> {
        self.auth.authenticate(token, now_logical).map(|_| ())
    }

    /// Opens a session after authentication.
    ///
    /// # Errors
    ///
    /// Missing, invalid, revoked, or expired tokens.
    pub fn connect(
        &mut self,
        token: Option<&str>,
        now_logical: u64,
    ) -> Result<SessionId, FanoutError> {
        let Some(token) = token.filter(|t| !t.is_empty()) else {
            return Err(FanoutError::MissingToken);
        };
        self.auth.authenticate(Some(token), now_logical)?;
        let token = token.to_owned();
        let id = SessionId::from_u64(self.next_id);
        self.next_id = self.next_id.saturating_add(1);
        self.sessions.insert(
            id,
            Session {
                token,
                subscriptions: HashSet::new(),
                outbound: VecDeque::new(),
                dropped: 0,
                last_activity_logical: now_logical,
                last_heartbeat_logical: now_logical,
                dead: false,
            },
        );
        Ok(id)
    }

    /// Drops a session and its outbound queue.
    pub fn disconnect(&mut self, id: SessionId) {
        self.sessions.remove(&id);
    }

    /// Returns true if the session exists and has not been marked dead.
    #[must_use]
    pub fn is_open(&self, id: SessionId) -> bool {
        self.sessions.get(&id).is_some_and(|s| !s.dead)
    }

    /// Connected session count (including dead-but-not-disconnected).
    #[must_use]
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    /// Cumulative dropped outbound frames for a session.
    #[must_use]
    pub fn dropped(&self, id: SessionId) -> Option<u64> {
        self.sessions.get(&id).map(|s| s.dropped)
    }

    /// Decodes and applies a client text frame.
    ///
    /// Recoverable errors enqueue an [`ClientMessage::Error`] and are returned.
    ///
    /// # Errors
    ///
    /// Unknown session, bad command, unknown instrument, or subscription cap.
    pub fn handle_text(
        &mut self,
        id: SessionId,
        raw: &[u8],
        now_logical: u64,
    ) -> Result<(), FanoutError> {
        match crate::protocol::decode_command(raw) {
            Ok(cmd) => self.handle_command(id, cmd, now_logical),
            Err(err) => {
                self.enqueue_error(id, err.code());
                Err(err)
            }
        }
    }

    /// Applies an already-decoded command.
    ///
    /// # Errors
    ///
    /// Unknown session, unknown instrument, or subscription cap.
    pub fn handle_command(
        &mut self,
        id: SessionId,
        command: ClientCommand,
        now_logical: u64,
    ) -> Result<(), FanoutError> {
        if self.sessions.get(&id).is_none_or(|s| s.dead) {
            return Err(FanoutError::UnknownSession);
        }
        match command {
            ClientCommand::Heartbeat => {
                self.touch(id, now_logical);
                Ok(())
            }
            ClientCommand::Subscribe { symbol } => self.subscribe(id, &symbol, now_logical),
            ClientCommand::Unsubscribe { symbol } => self.unsubscribe(id, &symbol, now_logical),
        }
    }

    /// Fans an event to live subscribers of its instrument. Overflow drops oldest.
    pub fn publish(&mut self, event: MarketEvent) {
        let instrument_id = event.instrument_id();
        let cap = self.config.queue_capacity;
        for session in self.sessions.values_mut() {
            if session.dead || !session.subscriptions.contains(&instrument_id) {
                continue;
            }
            session.enqueue(cap, ClientMessage::Market(event));
        }
    }

    /// Takes all pending outbound messages for a session.
    #[must_use]
    pub fn drain(&mut self, id: SessionId) -> Vec<ClientMessage> {
        self.sessions
            .get_mut(&id)
            .map(|s| s.outbound.drain(..).collect())
            .unwrap_or_default()
    }

    /// Heartbeats, idle TTL, and token revocation sweep.
    pub fn on_clock(&mut self, now_logical: u64) -> ClockOutcome {
        let cap = self.config.queue_capacity;
        let hb_every = self.config.heartbeat_every_logical;
        let ttl = self.config.session_ttl_logical;
        let mut closed = Vec::new();
        let ids: Vec<SessionId> = self.sessions.keys().copied().collect();
        for id in ids {
            let (revoked, idle, need_hb, dropped) = {
                let Some(session) = self.sessions.get(&id) else {
                    continue;
                };
                if session.dead {
                    continue;
                }
                (
                    self.auth.is_revoked(&session.token, now_logical),
                    now_logical.saturating_sub(session.last_activity_logical),
                    now_logical.saturating_sub(session.last_heartbeat_logical) >= hb_every,
                    session.dropped,
                )
            };
            let Some(session) = self.sessions.get_mut(&id) else {
                continue;
            };
            if revoked {
                session.enqueue(
                    cap,
                    ClientMessage::Error {
                        code: FanoutError::RevokedToken.code(),
                    },
                );
                session.dead = true;
                closed.push((id, CloseReason::Revoked));
                continue;
            }
            if idle >= ttl {
                session.enqueue(
                    cap,
                    ClientMessage::Error {
                        code: "session_expired",
                    },
                );
                session.dead = true;
                closed.push((id, CloseReason::Ttl));
                continue;
            }
            if need_hb {
                session.enqueue(
                    cap,
                    ClientMessage::Heartbeat {
                        ts_logical: now_logical,
                        dropped,
                    },
                );
                session.last_heartbeat_logical = now_logical;
            }
        }
        ClockOutcome { closed }
    }

    fn subscribe(
        &mut self,
        id: SessionId,
        symbol: &str,
        now_logical: u64,
    ) -> Result<(), FanoutError> {
        let instrument_id = match resolve_symbol(&self.master, symbol) {
            Ok(i) => i,
            Err(err) => {
                self.enqueue_error(id, err.code());
                return Err(err);
            }
        };
        let cap = self.config.queue_capacity;
        let max_subs = self.config.max_subscriptions;
        let session = self
            .sessions
            .get_mut(&id)
            .ok_or(FanoutError::UnknownSession)?;
        if session.dead {
            return Err(FanoutError::UnknownSession);
        }
        session.last_activity_logical = now_logical;
        if !session.subscriptions.contains(&instrument_id)
            && session.subscriptions.len() >= max_subs
        {
            session.enqueue(
                cap,
                ClientMessage::Error {
                    code: FanoutError::TooManySubscriptions.code(),
                },
            );
            return Err(FanoutError::TooManySubscriptions);
        }
        session.subscriptions.insert(instrument_id);
        session.enqueue(cap, ClientMessage::Subscribed { instrument_id });
        Ok(())
    }

    fn unsubscribe(
        &mut self,
        id: SessionId,
        symbol: &str,
        now_logical: u64,
    ) -> Result<(), FanoutError> {
        let instrument_id = match resolve_symbol(&self.master, symbol) {
            Ok(i) => i,
            Err(err) => {
                self.enqueue_error(id, err.code());
                return Err(err);
            }
        };
        self.touch(id, now_logical);
        if let Some(session) = self.sessions.get_mut(&id) {
            session.subscriptions.remove(&instrument_id);
        }
        Ok(())
    }

    fn touch(&mut self, id: SessionId, now_logical: u64) {
        if let Some(session) = self.sessions.get_mut(&id) {
            session.last_activity_logical = now_logical;
        }
    }

    fn enqueue_error(&mut self, id: SessionId, code: &'static str) {
        let cap = self.config.queue_capacity;
        if let Some(session) = self.sessions.get_mut(&id) {
            session.enqueue(cap, ClientMessage::Error { code });
        }
    }
}

/// Resolves a ticker or broker symbol via the instrument master.
pub fn resolve_symbol(
    master: &InstrumentMaster,
    symbol: &str,
) -> Result<InstrumentId, FanoutError> {
    let ticker = ExternalId::ticker(symbol).map_err(|_| FanoutError::UnknownInstrument)?;
    if let Ok(id) = master.resolve_alias(&ticker) {
        return Ok(id);
    }
    let broker = ExternalId::new(IdType::BrokerSymbol, symbol, None)
        .map_err(|_| FanoutError::UnknownInstrument)?;
    master
        .resolve_alias(&broker)
        .map_err(|_| FanoutError::UnknownInstrument)
}

impl<A> core::fmt::Debug for FanoutHub<A> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FanoutHub")
            .field("config", &self.config)
            .field("sessions", &self.sessions.len())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{StaticTokenAuth, SubjectId};
    use shinrai_instruments::{btc_usd, phase1_master, PriceTicks};
    use shinrai_market_data::{MdKind, MdRecord};

    fn hub_with(auth: StaticTokenAuth, cfg: FanoutConfig) -> FanoutHub<StaticTokenAuth> {
        FanoutHub::new(cfg, auth, phase1_master())
    }

    fn authed() -> (StaticTokenAuth, FanoutHub<StaticTokenAuth>) {
        let auth = StaticTokenAuth::new();
        auth.grant("t", SubjectId::new("alice"));
        let hub = hub_with(auth.clone(), FanoutConfig::default());
        (auth, hub)
    }

    fn trade(seq: u64) -> MarketEvent {
        MarketEvent::Tick(MdRecord::new(
            btc_usd().id(),
            seq,
            seq,
            MdKind::Trade,
            PriceTicks::from_scaled(6_500_000),
        ))
    }

    #[test]
    fn connect_requires_token() {
        let auth = StaticTokenAuth::new();
        let mut hub = hub_with(auth, FanoutConfig::default());
        assert_eq!(
            hub.connect(None, 0).expect_err("missing"),
            FanoutError::MissingToken
        );
        assert_eq!(
            hub.connect(Some("nope"), 0).expect_err("bad"),
            FanoutError::InvalidToken
        );
    }

    #[test]
    fn subscribe_and_publish_to_one_session() {
        let (_, mut hub) = authed();
        let a = hub.connect(Some("t"), 0).expect("connect");
        hub.handle_command(
            a,
            ClientCommand::Subscribe {
                symbol: "BTC-USD".into(),
            },
            1,
        )
        .expect("sub");
        let _ = hub.drain(a);
        hub.publish(trade(1));
        let out = hub.drain(a);
        assert_eq!(out.len(), 1);
        assert!(matches!(
            out[0],
            ClientMessage::Market(MarketEvent::Tick(_))
        ));
    }

    #[test]
    fn unsubscribe_stops_ticks() {
        let (_, mut hub) = authed();
        let a = hub.connect(Some("t"), 0).expect("connect");
        hub.handle_text(a, br#"{"type":"subscribe","symbol":"BTC-USD"}"#, 1)
            .expect("sub");
        let _ = hub.drain(a);
        hub.handle_text(a, br#"{"type":"unsubscribe","symbol":"BTC-USD"}"#, 2)
            .expect("unsub");
        hub.publish(trade(1));
        assert!(hub.drain(a).is_empty());
    }

    #[test]
    fn overflow_drops_oldest() {
        let auth = StaticTokenAuth::new();
        auth.grant("t", SubjectId::new("alice"));
        let mut hub = hub_with(auth, FanoutConfig::new(2, 16, 15, 45));
        let a = hub.connect(Some("t"), 0).expect("connect");
        hub.handle_command(
            a,
            ClientCommand::Subscribe {
                symbol: "BTC-USD".into(),
            },
            1,
        )
        .expect("sub");
        let _ = hub.drain(a);
        hub.publish(trade(1));
        hub.publish(trade(2));
        hub.publish(trade(3));
        assert_eq!(hub.dropped(a), Some(1));
        let out = hub.drain(a);
        assert_eq!(out.len(), 2);
        match &out[0] {
            ClientMessage::Market(MarketEvent::Tick(r)) => assert_eq!(r.seq(), 2),
            other => panic!("expected tick 2, got {other:?}"),
        }
        match &out[1] {
            ClientMessage::Market(MarketEvent::Tick(r)) => assert_eq!(r.seq(), 3),
            other => panic!("expected tick 3, got {other:?}"),
        }
    }

    #[test]
    fn subscription_cap() {
        let auth = StaticTokenAuth::new();
        auth.grant("t", SubjectId::new("alice"));
        let mut hub = hub_with(auth, FanoutConfig::new(64, 1, 15, 45));
        let a = hub.connect(Some("t"), 0).expect("connect");
        hub.handle_command(
            a,
            ClientCommand::Subscribe {
                symbol: "BTC-USD".into(),
            },
            1,
        )
        .expect("first");
        let err = hub
            .handle_command(
                a,
                ClientCommand::Subscribe {
                    symbol: "AAPL".into(),
                },
                2,
            )
            .expect_err("cap");
        assert_eq!(err, FanoutError::TooManySubscriptions);
        hub.handle_command(
            a,
            ClientCommand::Subscribe {
                symbol: "BTC-USD".into(),
            },
            3,
        )
        .expect("idempotent");
    }

    #[test]
    fn unknown_instrument_enqueues_error() {
        let (_, mut hub) = authed();
        let a = hub.connect(Some("t"), 0).expect("connect");
        let err = hub
            .handle_command(
                a,
                ClientCommand::Subscribe {
                    symbol: "NOPE".into(),
                },
                1,
            )
            .expect_err("unknown");
        assert_eq!(err, FanoutError::UnknownInstrument);
        let out = hub.drain(a);
        assert!(matches!(
            out[0],
            ClientMessage::Error {
                code: "unknown_instrument"
            }
        ));
    }

    #[test]
    fn heartbeat_includes_drop_count() {
        let auth = StaticTokenAuth::new();
        auth.grant("t", SubjectId::new("alice"));
        let mut hub = hub_with(auth, FanoutConfig::new(2, 16, 15, 45));
        let a = hub.connect(Some("t"), 0).expect("connect");
        hub.handle_command(
            a,
            ClientCommand::Subscribe {
                symbol: "BTC-USD".into(),
            },
            1,
        )
        .expect("sub");
        let _ = hub.drain(a);
        hub.publish(trade(1));
        hub.publish(trade(2));
        hub.publish(trade(3));
        let _ = hub.drain(a);
        let outcome = hub.on_clock(15);
        assert!(outcome.closed.is_empty());
        let out = hub.drain(a);
        assert!(matches!(
            out.last(),
            Some(ClientMessage::Heartbeat {
                ts_logical: 15,
                dropped: 1
            })
        ));
    }

    #[test]
    fn idle_ttl_marks_dead() {
        let (_, mut hub) = authed();
        let a = hub.connect(Some("t"), 0).expect("connect");
        let outcome = hub.on_clock(45);
        assert_eq!(outcome.closed, vec![(a, CloseReason::Ttl)]);
        assert!(!hub.is_open(a));
        let out = hub.drain(a);
        assert!(matches!(
            out.last(),
            Some(ClientMessage::Error {
                code: "session_expired"
            })
        ));
        hub.disconnect(a);
        assert_eq!(hub.session_count(), 0);
    }

    #[test]
    fn client_heartbeat_refreshes_ttl() {
        let (_, mut hub) = authed();
        let a = hub.connect(Some("t"), 0).expect("connect");
        hub.handle_command(a, ClientCommand::Heartbeat, 40)
            .expect("hb");
        let outcome = hub.on_clock(45);
        assert!(outcome.closed.is_empty());
        assert!(hub.is_open(a));
    }

    #[test]
    fn revoke_marks_dead_on_clock() {
        let (auth, mut hub) = authed();
        let a = hub.connect(Some("t"), 0).expect("connect");
        auth.revoke("t");
        let outcome = hub.on_clock(1);
        assert_eq!(outcome.closed, vec![(a, CloseReason::Revoked)]);
        assert!(!hub.is_open(a));
        let out = hub.drain(a);
        assert!(matches!(
            out.last(),
            Some(ClientMessage::Error { code: "revoked" })
        ));
    }

    #[test]
    fn expired_access_closes_within_ttl_window() {
        use crate::auth::{TokenAuth, TokenTtl};
        let auth = TokenAuth::new(TokenTtl::new(60, 3_600));
        auth.register_client("cli", "sec", SubjectId::new("alice"));
        let pair = auth
            .issue_client_credentials("cli", "sec", 100)
            .expect("issue");
        // Long idle TTL so expiry (not idle) is what closes the session.
        let mut hub = FanoutHub::new(FanoutConfig::new(64, 16, 15, 10_000), auth, phase1_master());
        let sid = hub
            .connect(Some(pair.access_token()), 100)
            .expect("connect");
        assert!(hub.on_clock(160).closed.is_empty());
        let closed = hub.on_clock(161);
        assert_eq!(closed.closed, vec![(sid, CloseReason::Revoked)]);
    }

    #[test]
    fn second_session_without_sub_gets_nothing() {
        let auth = StaticTokenAuth::new();
        auth.grant("t", SubjectId::new("alice"));
        auth.grant("u", SubjectId::new("bob"));
        let mut hub = hub_with(auth, FanoutConfig::default());
        let a = hub.connect(Some("t"), 0).expect("a");
        let b = hub.connect(Some("u"), 0).expect("b");
        hub.handle_command(
            a,
            ClientCommand::Subscribe {
                symbol: "BTCUSD".into(),
            },
            1,
        )
        .expect("sub");
        let _ = hub.drain(a);
        hub.publish(trade(1));
        assert_eq!(hub.drain(a).len(), 1);
        assert!(hub.drain(b).is_empty());
    }

    #[test]
    fn debug_hub_does_not_leak_token() {
        let auth = StaticTokenAuth::new();
        auth.grant("super-secret-token", SubjectId::new("alice"));
        let mut hub = hub_with(auth, FanoutConfig::default());
        let _ = hub.connect(Some("super-secret-token"), 0).expect("connect");
        let rendered = format!("{hub:?}");
        assert!(!rendered.contains("super-secret-token"));
    }
}
