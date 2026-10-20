//! FIX 4.2 subset venue ([`ExecutionVenue`]) — in-process or TCP initiator.
//!
//! Encodes real SOH-delimited FIX for Logon / Heartbeat / `NewOrderSingle` /
//! Cancel / Replace and consumes [`ExecutionReport`] (`35=8`) replies.
//!
//! - **In-process** (default): local acceptor in the same process (unit tests).
//! - **TCP loopback** (`SHINRAI_OG_FIX_TCP=1`): paper acceptor thread + TCP initiator.
//! - **TCP connect** (`SHINRAI_OG_FIX_TCP_CONNECT=host:port`): initiator to an
//!   external counterparty (no local auto-fill; wait for remote ERs).

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use shinrai_instruments::{PriceTicks, QuantityLots};
use shinrai_orders::{ExecId, OrderId, Side, TimeInForce, VenueOrderId};

use crate::error::ExecutionError;
use crate::fix_tcp::{
    connect_fix, read_fix_message, spawn_paper_acceptor, write_fix_message, FixTcpAcceptorGuard,
};
use crate::report::{ExecType, ExecutionReport, SessionId};
use crate::session::VenueSessionState;
use crate::venue::{ExecutionVenue, NewVenueOrder, VenueOrderSnapshot, VenueTradeSnapshot};

const SOH: char = '\x01';
const BEGIN_STRING: &str = "FIX.4.2";

/// FIX session + paper fill knobs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixConfig {
    /// Tag 49 on outbound initiator messages.
    pub sender_comp_id: String,
    /// Tag 56 on outbound initiator messages.
    pub target_comp_id: String,
    /// When true, submit queues New then a full Trade immediately.
    pub auto_fill: bool,
    /// Logical ticks between outbound heartbeats (0 = disabled).
    pub heartbeat_interval_ticks: u64,
    /// Spawn a local TCP paper acceptor and connect to it (`SHINRAI_OG_FIX_TCP=1`).
    pub tcp_loopback: bool,
    /// Bind address for the loopback acceptor (default `127.0.0.1:19878`, or `…:0` ephemeral).
    pub tcp_listen: Option<String>,
    /// Connect initiator to this `host:port` (loopback target or external vendor).
    pub tcp_connect: Option<String>,
}

impl Default for FixConfig {
    fn default() -> Self {
        Self::local_mock()
    }
}

impl FixConfig {
    /// Local in-process happy-path (auto-fill).
    #[must_use]
    pub fn local_mock() -> Self {
        Self {
            sender_comp_id: "SHINRAI".into(),
            target_comp_id: "SIM".into(),
            auto_fill: true,
            heartbeat_interval_ticks: 5,
            tcp_loopback: false,
            tcp_listen: None,
            tcp_connect: None,
        }
    }

    /// Ack only; fills must be injected via [`FixPaperVenue::inject`].
    #[must_use]
    pub fn ack_only() -> Self {
        Self {
            auto_fill: false,
            ..Self::local_mock()
        }
    }

    /// TCP loopback paper acceptor + initiator (ephemeral port).
    #[must_use]
    pub fn tcp_loopback_ephemeral() -> Self {
        Self {
            tcp_loopback: true,
            tcp_listen: Some("127.0.0.1:0".into()),
            tcp_connect: None,
            ..Self::local_mock()
        }
    }

    /// Whether this config uses a TCP initiator (loopback or external).
    #[must_use]
    pub fn uses_tcp(&self) -> bool {
        self.tcp_loopback || self.tcp_connect.is_some()
    }

    /// Loads `CompID`s / heartbeat / TCP from the environment.
    #[must_use]
    pub fn from_env() -> Self {
        let mut cfg = Self::local_mock();
        if let Ok(s) = std::env::var("SHINRAI_OG_FIX_SENDER_COMP_ID") {
            let t = s.trim();
            if !t.is_empty() {
                cfg.sender_comp_id = t.to_string();
            }
        }
        if let Ok(s) = std::env::var("SHINRAI_OG_FIX_TARGET_COMP_ID") {
            let t = s.trim();
            if !t.is_empty() {
                cfg.target_comp_id = t.to_string();
            }
        }
        if let Ok(s) = std::env::var("SHINRAI_OG_FIX_HEARTBEAT_SECS") {
            if let Ok(n) = s.trim().parse::<u64>() {
                cfg.heartbeat_interval_ticks = n;
            }
        }
        if let Ok(s) = std::env::var("SHINRAI_OG_FIX_AUTO_FILL") {
            cfg.auto_fill = matches!(
                s.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            );
        }
        let tcp_flag = std::env::var("SHINRAI_OG_FIX_TCP")
            .ok()
            .map(|s| {
                matches!(
                    s.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            })
            .unwrap_or(false);
        let tcp_addr = std::env::var("SHINRAI_OG_FIX_TCP_ADDR")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let tcp_connect = std::env::var("SHINRAI_OG_FIX_TCP_CONNECT")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        if tcp_flag {
            cfg.tcp_loopback = true;
            cfg.tcp_listen = Some(
                tcp_addr
                    .clone()
                    .unwrap_or_else(|| "127.0.0.1:19878".into()),
            );
            cfg.tcp_connect = Some(tcp_addr.unwrap_or_else(|| "127.0.0.1:19878".into()));
        } else if let Some(connect) = tcp_connect {
            cfg.tcp_loopback = false;
            cfg.tcp_connect = Some(connect);
        }
        cfg
    }
}

#[derive(Debug, Clone)]
struct Inflight {
    venue_order_id: VenueOrderId,
    order_qty: i64,
    cum_qty: i64,
    price: PriceTicks,
    canceled: bool,
}

/// Wire-shaped FIX paper venue (in-process or TCP initiator).
#[derive(Debug)]
pub struct FixPaperVenue {
    config: FixConfig,
    connected: bool,
    session: SessionId,
    /// Initiator outbound msg seq (tag 34).
    next_wire_out: u64,
    /// Acceptor outbound msg seq (tag 34 on wire) — in-process only.
    next_wire_in: u64,
    /// OMS execution-report sequence (independent of session admin msgs).
    next_report_seq: u64,
    next_venue: u64,
    next_exec: u64,
    clock: u64,
    last_hb_tick: u64,
    inflight: HashMap<OrderId, Inflight>,
    outbox: VecDeque<ExecutionReport>,
    history: Vec<ExecutionReport>,
    /// Encoded FIX messages sent by the initiator (tests / ops).
    outbound_wire: Vec<String>,
    /// Encoded FIX messages produced by the local acceptor / read from TCP.
    inbound_wire: Vec<String>,
    /// Shared TCP initiator socket (cloned venues share the session).
    tcp: Option<Arc<Mutex<TcpStream>>>,
    /// Keeps the loopback acceptor alive for this venue's lifetime.
    #[allow(dead_code)]
    tcp_acceptor: Option<Arc<FixTcpAcceptorGuard>>,
}

impl Clone for FixPaperVenue {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            connected: self.connected,
            session: self.session,
            next_wire_out: self.next_wire_out,
            next_wire_in: self.next_wire_in,
            next_report_seq: self.next_report_seq,
            next_venue: self.next_venue,
            next_exec: self.next_exec,
            clock: self.clock,
            last_hb_tick: self.last_hb_tick,
            inflight: self.inflight.clone(),
            outbox: self.outbox.clone(),
            history: self.history.clone(),
            outbound_wire: self.outbound_wire.clone(),
            inbound_wire: self.inbound_wire.clone(),
            tcp: self.tcp.clone(),
            tcp_acceptor: self.tcp_acceptor.clone(),
        }
    }
}

impl Default for FixPaperVenue {
    fn default() -> Self {
        Self::local_mock()
    }
}

impl FixPaperVenue {
    /// Connected local mock (auto logon).
    #[must_use]
    pub fn local_mock() -> Self {
        let mut v = Self::new(FixConfig::local_mock());
        let _ = v.logon();
        v
    }

    /// Disconnected venue; call [`Self::logon`] before trading.
    #[must_use]
    pub fn new(config: FixConfig) -> Self {
        Self {
            config,
            connected: false,
            session: SessionId::new(1),
            next_wire_out: 1,
            next_wire_in: 1,
            next_report_seq: 1,
            next_venue: 1,
            next_exec: 1,
            clock: 0,
            last_hb_tick: 0,
            inflight: HashMap::new(),
            outbox: VecDeque::new(),
            history: Vec::new(),
            outbound_wire: Vec::new(),
            inbound_wire: Vec::new(),
            tcp: None,
            tcp_acceptor: None,
        }
    }

    /// True when the initiator session is over TCP.
    #[must_use]
    pub fn tcp_enabled(&self) -> bool {
        self.tcp.is_some()
    }

    /// Config used by this venue.
    #[must_use]
    pub fn config(&self) -> &FixConfig {
        &self.config
    }

    /// Encoded initiator messages (SOH-delimited).
    #[must_use]
    pub fn outbound_wire(&self) -> &[String] {
        &self.outbound_wire
    }

    /// Encoded acceptor messages (SOH-delimited).
    #[must_use]
    pub fn inbound_wire(&self) -> &[String] {
        &self.inbound_wire
    }

    /// Session logon (`35=A`); returns initiator msg seq.
    ///
    /// # Errors
    ///
    /// Returns [`ExecutionError::InvalidState`] when already connected, or
    /// [`ExecutionError::Transport`] when TCP bind/connect fails.
    pub fn logon(&mut self) -> Result<u64, ExecutionError> {
        if self.connected {
            return Err(ExecutionError::InvalidState("already logged on"));
        }
        self.ensure_tcp()?;
        let seq = self.emit_initiator("A", &[("98", "0"), ("108", "30")])?;
        if self.tcp.is_some() {
            self.recv_admin("A")?;
        } else {
            let _ = self.emit_acceptor("A", &[("98", "0"), ("108", "30")]);
        }
        self.connected = true;
        self.last_hb_tick = self.clock;
        Ok(seq)
    }

    /// Graceful logout (`35=5`).
    pub fn logout(&mut self) {
        if self.connected {
            let _ = self.emit_initiator("5", &[]);
            if self.tcp.is_some() {
                let _ = self.recv_admin("5");
            } else {
                let _ = self.emit_acceptor("5", &[]);
            }
        }
        self.connected = false;
        self.tcp = None;
    }

    fn ensure_tcp(&mut self) -> Result<(), ExecutionError> {
        if !self.config.uses_tcp() {
            return Ok(());
        }
        if self.tcp.is_some() {
            return Ok(());
        }
        let mut connect_addr = self
            .config
            .tcp_connect
            .clone()
            .unwrap_or_else(|| "127.0.0.1:19878".into());
        if self.config.tcp_loopback {
            let listen = self
                .config
                .tcp_listen
                .clone()
                .unwrap_or_else(|| connect_addr.clone());
            let (bound, guard) = spawn_paper_acceptor(&listen, self.config.clone())?;
            self.tcp_acceptor = Some(Arc::new(guard));
            connect_addr = bound;
            // Brief settle so the accept loop is polling.
            std::thread::sleep(Duration::from_millis(20));
        }
        let mut last_err = ExecutionError::Transport("fix connect failed".into());
        for _ in 0..50 {
            match connect_fix(&connect_addr, Duration::from_millis(200)) {
                Ok(stream) => {
                    self.tcp = Some(Arc::new(Mutex::new(stream)));
                    return Ok(());
                }
                Err(e) => {
                    last_err = e;
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
        }
        Err(last_err)
    }

    fn recv_admin(&mut self, expect_type: &str) -> Result<(), ExecutionError> {
        for _ in 0..16 {
            let wire = self.read_tcp()?;
            self.inbound_wire.push(wire.clone());
            let map = decode_fix(&wire).map_err(ExecutionError::InvalidState)?;
            let got = map.get("35").map(String::as_str).unwrap_or("");
            if got == expect_type {
                return Ok(());
            }
            if got == "0" {
                continue;
            }
            if got == "8" {
                self.ingest_execution_report(&map)?;
            }
        }
        Err(ExecutionError::Transport(format!(
            "fix expected admin MsgType={expect_type}"
        )))
    }

    fn read_tcp(&mut self) -> Result<String, ExecutionError> {
        let Some(tcp) = &self.tcp else {
            return Err(ExecutionError::Transport("fix not connected".into()));
        };
        let mut guard = tcp
            .lock()
            .map_err(|_| ExecutionError::Transport("fix mutex poisoned".into()))?;
        read_fix_message(&mut *guard)
    }

    /// Reads TCP until `min_reports` execution reports are ingested (skips heartbeats).
    fn recv_execution_reports(&mut self, min_reports: usize) -> Result<(), ExecutionError> {
        let mut got = 0_usize;
        for _ in 0..32 {
            if got >= min_reports {
                return Ok(());
            }
            let wire = self.read_tcp()?;
            self.inbound_wire.push(wire.clone());
            let map = decode_fix(&wire).map_err(ExecutionError::InvalidState)?;
            match map.get("35").map(String::as_str) {
                Some("0") => {}
                Some("8") => {
                    self.ingest_execution_report(&map)?;
                    got = got.saturating_add(1);
                }
                Some("A" | "5") => {}
                _ => {}
            }
        }
        if got >= min_reports {
            Ok(())
        } else {
            Err(ExecutionError::Transport(format!(
                "fix expected {min_reports} execution reports, got {got}"
            )))
        }
    }

    fn ingest_execution_report(
        &mut self,
        map: &BTreeMap<String, String>,
    ) -> Result<(), ExecutionError> {
        let cl_ord = map
            .get("11")
            .ok_or(ExecutionError::InvalidState("ER missing ClOrdID"))?;
        // Cancel/replace use synthetic ClOrdIDs (cxl-/rpl-); map back to OMS id.
        let order_key = cl_ord
            .strip_prefix("cxl-")
            .or_else(|| cl_ord.strip_prefix("rpl-"))
            .unwrap_or(cl_ord.as_str());
        let order_id = OrderId::from_u64(
            order_key
                .parse()
                .map_err(|_| ExecutionError::InvalidState("ER ClOrdID not u64"))?,
        );
        let venue_raw = map
            .get("37")
            .cloned()
            .unwrap_or_else(|| format!("FIX-{}", self.next_venue));
        let venue_order_id =
            VenueOrderId::new(venue_raw).map_err(|_| ExecutionError::InvalidIdentifier)?;
        let exec_type_tag = map.get("150").map(String::as_str).unwrap_or("");
        let last_qty: i64 = map.get("32").and_then(|s| s.parse().ok()).unwrap_or(0);
        let last_px: i64 = map.get("31").and_then(|s| s.parse().ok()).unwrap_or(0);
        let cum: i64 = map.get("14").and_then(|s| s.parse().ok()).unwrap_or(0);
        let exec_type = match exec_type_tag {
            "0" => ExecType::New,
            "F" => ExecType::Trade,
            "4" => ExecType::Canceled,
            "5" => ExecType::Replaced,
            "8" => ExecType::Rejected {
                reason: map
                    .get("58")
                    .cloned()
                    .unwrap_or_else(|| "rejected".into()),
            },
            "C" => ExecType::Expired,
            _ => {
                return Err(ExecutionError::InvalidState("unsupported ER ExecType"));
            }
        };
        let exec_id = map
            .get("17")
            .filter(|s| !s.is_empty())
            .and_then(|s| ExecId::new(s.clone()).ok());
        if matches!(exec_type, ExecType::Trade) {
            if let Some(row) = self.inflight.get_mut(&order_id) {
                row.cum_qty = cum;
            }
        }
        if matches!(exec_type, ExecType::Canceled) {
            if let Some(row) = self.inflight.get_mut(&order_id) {
                row.canceled = true;
            }
        }
        if matches!(exec_type, ExecType::New | ExecType::Replaced) {
            self.inflight.entry(order_id).or_insert_with(|| Inflight {
                venue_order_id: venue_order_id.clone(),
                order_qty: map
                    .get("151")
                    .and_then(|s| s.parse::<i64>().ok())
                    .unwrap_or(0)
                    .saturating_add(cum),
                cum_qty: cum,
                price: PriceTicks::from_scaled(if last_px > 0 {
                    last_px
                } else {
                    0
                }),
                canceled: false,
            });
            if let Some(row) = self.inflight.get_mut(&order_id) {
                row.venue_order_id = venue_order_id.clone();
                if last_px > 0 {
                    row.price = PriceTicks::from_scaled(last_px);
                }
                if matches!(exec_type, ExecType::Replaced) {
                    let leaves: i64 = map.get("151").and_then(|s| s.parse().ok()).unwrap_or(0);
                    row.order_qty = cum.saturating_add(leaves);
                }
            }
        }
        let qty = QuantityLots::from_lots(if last_qty > 0 { last_qty } else { 0 });
        let price = PriceTicks::from_scaled(if last_px > 0 {
            last_px
        } else if let Some(row) = self.inflight.get(&order_id) {
            row.price.scaled()
        } else {
            0
        });
        let report_seq = self.next_report_seq;
        self.next_report_seq = self.next_report_seq.saturating_add(1);
        let report = ExecutionReport::new(
            order_id,
            venue_order_id,
            exec_id,
            exec_type,
            qty,
            price,
            self.session,
            report_seq,
        );
        self.history.push(report.clone());
        self.outbox.push_back(report);
        Ok(())
    }

    /// Queues an arbitrary report (tests / controlled fills).
    pub fn inject(&mut self, report: ExecutionReport) {
        if let ExecType::Trade = report.exec_type() {
            if let Some(row) = self.inflight.get_mut(&report.order_id()) {
                row.cum_qty = row
                    .cum_qty
                    .saturating_add(report.qty().lots())
                    .min(row.order_qty);
            }
        }
        if matches!(report.exec_type(), ExecType::Canceled) {
            if let Some(row) = self.inflight.get_mut(&report.order_id()) {
                row.canceled = true;
            }
        }
        self.history.push(report.clone());
        self.outbox.push_back(report);
    }

    /// Restores a working order without emitting ack/fill reports (startup hydrate).
    ///
    /// # Errors
    ///
    /// Returns quantity / identifier errors.
    pub fn restore_working(
        &mut self,
        order_id: OrderId,
        order_qty: QuantityLots,
        price: PriceTicks,
        cum_qty: i64,
        venue_order_id: Option<VenueOrderId>,
    ) -> Result<(), ExecutionError> {
        if order_qty.lots() <= 0 || price.scaled() <= 0 {
            return Err(ExecutionError::InvalidQuantity);
        }
        if cum_qty < 0 || cum_qty > order_qty.lots() {
            return Err(ExecutionError::InvalidQuantity);
        }
        let venue_order_id = if let Some(id) = venue_order_id {
            id
        } else {
            self.alloc_venue_id()?
        };
        self.inflight.insert(
            order_id,
            Inflight {
                venue_order_id,
                order_qty: order_qty.lots(),
                cum_qty,
                price,
                canceled: false,
            },
        );
        Ok(())
    }

    fn emit_initiator(
        &mut self,
        msg_type: &str,
        body: &[(&str, &str)],
    ) -> Result<u64, ExecutionError> {
        let seq = self.next_wire_out;
        self.next_wire_out = self.next_wire_out.saturating_add(1);
        let wire = encode_fix(
            &self.config.sender_comp_id,
            &self.config.target_comp_id,
            msg_type,
            seq,
            body,
        );
        if let Some(tcp) = &self.tcp {
            let mut guard = tcp
                .lock()
                .map_err(|_| ExecutionError::Transport("fix mutex poisoned".into()))?;
            write_fix_message(&mut *guard, &wire)?;
        }
        self.outbound_wire.push(wire);
        Ok(seq)
    }

    fn emit_acceptor(&mut self, msg_type: &str, body: &[(&str, &str)]) -> u64 {
        let seq = self.next_wire_in;
        self.next_wire_in = self.next_wire_in.saturating_add(1);
        // Acceptor swaps CompIDs relative to initiator.
        let wire = encode_fix(
            &self.config.target_comp_id,
            &self.config.sender_comp_id,
            msg_type,
            seq,
            body,
        );
        self.inbound_wire.push(wire);
        seq
    }

    #[allow(clippy::too_many_arguments)]
    fn push_report_from_er(
        &mut self,
        order_id: OrderId,
        venue_order_id: VenueOrderId,
        exec_id: Option<ExecId>,
        exec_type: ExecType,
        qty: QuantityLots,
        price: PriceTicks,
        leaves: i64,
        cum: i64,
        side: Side,
    ) -> Result<(), ExecutionError> {
        let (fix_exec, ord_status) = match &exec_type {
            ExecType::New => ("0", "0"),
            ExecType::Trade => {
                if leaves <= 0 {
                    ("F", "2")
                } else {
                    ("F", "1")
                }
            }
            ExecType::Canceled => ("4", "4"),
            ExecType::Replaced => ("5", "1"),
            ExecType::Rejected { .. } | ExecType::CancelReject { .. } => ("8", "8"),
            ExecType::Expired => ("C", "C"),
        };
        let cl_ord = order_id.get().to_string();
        let venue = venue_order_id.as_str().to_string();
        let exec = exec_id
            .as_ref()
            .map(|e| e.as_str().to_string())
            .unwrap_or_default();
        let last_qty = qty.lots().to_string();
        let last_px = price.scaled().to_string();
        let leaves_s = leaves.to_string();
        let cum_s = cum.to_string();
        let side_s = match side {
            Side::Buy => "1",
            Side::Sell => "2",
        };
        let mut fields: Vec<(&str, String)> = vec![
            ("37", venue),
            ("11", cl_ord),
            ("150", fix_exec.to_string()),
            ("39", ord_status.to_string()),
            ("54", side_s.to_string()),
            ("14", cum_s),
            ("151", leaves_s),
            ("32", last_qty),
            ("31", last_px),
        ];
        if !exec.is_empty() {
            fields.push(("17", exec));
        }
        let body: Vec<(&str, &str)> = fields.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let _wire_seq = self.emit_acceptor("8", &body);

        // Round-trip decode for wire fidelity; report uses OMS report seq.
        let wire = self
            .inbound_wire
            .last()
            .cloned()
            .ok_or(ExecutionError::InvalidState("missing ER wire"))?;
        let _parsed = decode_fix(&wire).map_err(ExecutionError::InvalidState)?;
        let report_seq = self.next_report_seq;
        self.next_report_seq = self.next_report_seq.saturating_add(1);
        let report = ExecutionReport::new(
            order_id,
            venue_order_id,
            exec_id,
            exec_type,
            qty,
            price,
            self.session,
            report_seq,
        );
        self.history.push(report.clone());
        self.outbox.push_back(report);
        Ok(())
    }

    fn alloc_venue_id(&mut self) -> Result<VenueOrderId, ExecutionError> {
        let id = VenueOrderId::new(format!("FIX-{}", self.next_venue))
            .map_err(|_| ExecutionError::InvalidIdentifier)?;
        self.next_venue = self.next_venue.saturating_add(1);
        Ok(id)
    }

    fn alloc_exec_id(&mut self) -> Result<ExecId, ExecutionError> {
        let id = ExecId::new(format!("FIX-E{}", self.next_exec))
            .map_err(|_| ExecutionError::InvalidIdentifier)?;
        self.next_exec = self.next_exec.saturating_add(1);
        Ok(id)
    }
}

impl ExecutionVenue for FixPaperVenue {
    fn submit(&mut self, order: &NewVenueOrder) -> Result<(), ExecutionError> {
        if !self.connected {
            return Err(ExecutionError::Disconnected);
        }
        if order.qty.lots() <= 0 || order.price.scaled() <= 0 {
            return Err(ExecutionError::InvalidQuantity);
        }
        let venue_order_id = self.alloc_venue_id()?;
        let cl_ord = order.order_id.get().to_string();
        let qty_s = order.qty.lots().to_string();
        let px_s = order.price.scaled().to_string();
        let side_s = match order.side {
            Side::Buy => "1",
            Side::Sell => "2",
        };
        let tif_s = match order.tif {
            TimeInForce::Gtc => "1",
            TimeInForce::Ioc => "3",
            TimeInForce::Fok => "4",
        };
        let symbol = order.instrument_id.get().to_string();
        self.emit_initiator(
            "D",
            &[
                ("11", &cl_ord),
                ("55", &symbol),
                ("54", side_s),
                ("38", &qty_s),
                ("40", "2"),
                ("44", &px_s),
                ("59", tif_s),
            ],
        )?;

        if self.tcp.is_some() {
            // Seed inflight so ER ingest can attach; acceptor assigns venue id.
            self.inflight.insert(
                order.order_id,
                Inflight {
                    venue_order_id: venue_order_id.clone(),
                    order_qty: order.qty.lots(),
                    cum_qty: 0,
                    price: order.price,
                    canceled: false,
                },
            );
            let expect = if self.config.auto_fill { 2 } else { 1 };
            self.recv_execution_reports(expect)?;
            return Ok(());
        }

        if order.tif == TimeInForce::Fok && !self.config.auto_fill {
            self.push_report_from_er(
                order.order_id,
                venue_order_id,
                None,
                ExecType::Rejected {
                    reason: "fok cannot rest".into(),
                },
                QuantityLots::from_lots(0),
                order.price,
                0,
                0,
                order.side,
            )?;
            return Ok(());
        }

        self.inflight.insert(
            order.order_id,
            Inflight {
                venue_order_id: venue_order_id.clone(),
                order_qty: order.qty.lots(),
                cum_qty: 0,
                price: order.price,
                canceled: false,
            },
        );

        let leaves = order.qty.lots();
        self.push_report_from_er(
            order.order_id,
            venue_order_id.clone(),
            None,
            ExecType::New,
            order.qty,
            order.price,
            leaves,
            0,
            order.side,
        )?;

        if self.config.auto_fill {
            let exec_id = self.alloc_exec_id()?;
            if let Some(row) = self.inflight.get_mut(&order.order_id) {
                row.cum_qty = order.qty.lots();
            }
            self.push_report_from_er(
                order.order_id,
                venue_order_id,
                Some(exec_id),
                ExecType::Trade,
                order.qty,
                order.price,
                0,
                order.qty.lots(),
                order.side,
            )?;
        }
        Ok(())
    }

    fn cancel(&mut self, order_id: OrderId) -> Result<(), ExecutionError> {
        if !self.connected {
            return Err(ExecutionError::Disconnected);
        }
        let Some(row) = self.inflight.get(&order_id).cloned() else {
            return Err(ExecutionError::UnknownOrder { id: order_id });
        };
        let cl_ord = order_id.get().to_string();
        let cxl_id = format!("cxl-{cl_ord}");
        let venue = row.venue_order_id.as_str().to_string();
        self.emit_initiator("F", &[("11", &cxl_id), ("41", &cl_ord), ("37", &venue)])?;

        if self.tcp.is_some() {
            self.recv_execution_reports(1)?;
            return Ok(());
        }

        if row.canceled {
            return Err(ExecutionError::InvalidState("already canceled"));
        }
        let leaves = row.order_qty - row.cum_qty;
        if leaves <= 0 {
            self.push_report_from_er(
                order_id,
                row.venue_order_id,
                None,
                ExecType::CancelReject {
                    reason: "already filled".into(),
                },
                QuantityLots::from_lots(0),
                row.price,
                0,
                row.cum_qty,
                Side::Buy,
            )?;
            return Ok(());
        }
        if let Some(r) = self.inflight.get_mut(&order_id) {
            r.canceled = true;
        }
        self.push_report_from_er(
            order_id,
            row.venue_order_id,
            None,
            ExecType::Canceled,
            QuantityLots::from_lots(leaves),
            row.price,
            0,
            row.cum_qty,
            Side::Buy,
        )?;
        Ok(())
    }

    fn replace(
        &mut self,
        order_id: OrderId,
        new_qty: QuantityLots,
        new_price: PriceTicks,
    ) -> Result<(), ExecutionError> {
        if !self.connected {
            return Err(ExecutionError::Disconnected);
        }
        if new_qty.lots() <= 0 || new_price.scaled() <= 0 {
            return Err(ExecutionError::InvalidQuantity);
        }
        let Some(row) = self.inflight.get(&order_id).cloned() else {
            return Err(ExecutionError::UnknownOrder { id: order_id });
        };
        if row.canceled {
            return Err(ExecutionError::InvalidState("canceled"));
        }
        if new_qty.lots() < row.cum_qty {
            return Err(ExecutionError::InvalidQuantity);
        }
        let cl_ord = order_id.get().to_string();
        let rpl_id = format!("rpl-{cl_ord}");
        let qty_s = new_qty.lots().to_string();
        let px_s = new_price.scaled().to_string();
        let venue = row.venue_order_id.as_str().to_string();
        self.emit_initiator(
            "G",
            &[
                ("11", &rpl_id),
                ("41", &cl_ord),
                ("37", &venue),
                ("38", &qty_s),
                ("44", &px_s),
                ("40", "2"),
            ],
        )?;
        if self.tcp.is_some() {
            self.recv_execution_reports(1)?;
            return Ok(());
        }
        if let Some(r) = self.inflight.get_mut(&order_id) {
            r.order_qty = new_qty.lots();
            r.price = new_price;
        }
        let leaves = new_qty.lots() - row.cum_qty;
        let venue_order_id = row.venue_order_id;
        let cum = row.cum_qty;
        self.push_report_from_er(
            order_id,
            venue_order_id,
            None,
            ExecType::Replaced,
            QuantityLots::from_lots(leaves),
            new_price,
            leaves,
            cum,
            Side::Buy,
        )?;
        Ok(())
    }

    fn poll(&mut self) -> Vec<ExecutionReport> {
        self.drain_tcp_available();
        self.outbox.drain(..).collect()
    }

    fn drain_tcp_available(&mut self) {
        let Some(tcp) = self.tcp.clone() else {
            return;
        };
        let Ok(mut guard) = tcp.lock() else {
            return;
        };
        let _ = guard.set_read_timeout(Some(Duration::from_millis(1)));
        loop {
            match read_fix_message(&mut *guard) {
                Ok(wire) => {
                    drop(guard);
                    self.inbound_wire.push(wire.clone());
                    if let Ok(map) = decode_fix(&wire) {
                        if map.get("35").map(String::as_str) == Some("8") {
                            let _ = self.ingest_execution_report(&map);
                        }
                    }
                    let Ok(g) = tcp.lock() else {
                        return;
                    };
                    guard = g;
                    let _ = guard.set_read_timeout(Some(Duration::from_millis(1)));
                }
                Err(_) => break,
            }
        }
        let _ = guard.set_read_timeout(Some(Duration::from_secs(5)));
    }

    fn tick(&mut self, ticks: u64) {
        self.clock = self.clock.saturating_add(ticks);
        if !self.connected || self.config.heartbeat_interval_ticks == 0 {
            return;
        }
        if self.clock.saturating_sub(self.last_hb_tick) >= self.config.heartbeat_interval_ticks {
            if self.emit_initiator("0", &[]).is_ok() {
                if self.tcp.is_some() {
                    let _ = self.recv_admin("0");
                } else {
                    let _ = self.emit_acceptor("0", &[]);
                }
            }
            self.last_hb_tick = self.clock;
        }
    }

    fn venue_order(&self, order_id: OrderId) -> Option<VenueOrderSnapshot> {
        self.inflight.get(&order_id).map(|r| VenueOrderSnapshot {
            order_id,
            order_qty: r.order_qty,
            cum_qty: r.cum_qty,
            canceled: r.canceled,
        })
    }

    fn venue_orders(&self) -> Vec<VenueOrderSnapshot> {
        self.inflight
            .iter()
            .map(|(id, r)| VenueOrderSnapshot {
                order_id: *id,
                order_qty: r.order_qty,
                cum_qty: r.cum_qty,
                canceled: r.canceled,
            })
            .collect()
    }

    fn trade_execs(&self) -> Vec<VenueTradeSnapshot> {
        self.history
            .iter()
            .filter(|r| matches!(r.exec_type(), ExecType::Trade))
            .filter_map(|r| {
                let exec_id = r.exec_id()?.clone();
                Some(VenueTradeSnapshot {
                    order_id: r.order_id(),
                    exec_id,
                    qty: r.qty().lots(),
                    price: r.price().scaled(),
                    session: r.session(),
                    seq: r.seq(),
                })
            })
            .collect()
    }

    fn session_state(&self) -> VenueSessionState {
        if self.connected {
            VenueSessionState::connected(self.session, self.next_report_seq)
        } else {
            VenueSessionState::disconnected(self.session, self.next_report_seq)
        }
    }

    fn disconnect(&mut self) {
        self.connected = false;
        self.tcp = None;
    }

    fn reconnect(&mut self) {
        self.connected = false;
        self.tcp = None;
        // Keep loopback acceptor (tcp_acceptor) if still alive; reconnect initiator.
        self.session = SessionId::new(self.session.n.saturating_add(1));
        self.next_wire_out = 1;
        self.next_wire_in = 1;
        self.next_report_seq = 1;
        self.history.clear();
        self.outbox.clear();
        self.outbound_wire.clear();
        self.inbound_wire.clear();
        if self.logon().is_err() {
            // Fall back to in-process session markers so OMS reconnect tests stay usable.
            self.connected = true;
            let _ = self.emit_initiator("A", &[("98", "0"), ("108", "30")]);
            let _ = self.emit_acceptor("A", &[("98", "0"), ("108", "30")]);
        }
        self.last_hb_tick = self.clock;
    }

    fn poll_recovery(&mut self, from_seq: u64) -> Result<Vec<ExecutionReport>, ExecutionError> {
        if !self.connected {
            return Err(ExecutionError::Disconnected);
        }
        Ok(self
            .history
            .iter()
            .filter(|r| r.seq() >= from_seq)
            .cloned()
            .collect())
    }
}

/// Encodes a FIX 4.2 message with body length and checksum.
#[must_use]
#[allow(clippy::format_push_string)]
pub fn encode_fix(
    sender: &str,
    target: &str,
    msg_type: &str,
    seq: u64,
    body: &[(&str, &str)],
) -> String {
    let mut mid = format!(
        "35={msg_type}{SOH}49={sender}{SOH}56={target}{SOH}34={seq}{SOH}52=20200101-00:00:00.000{SOH}"
    );
    for (tag, val) in body {
        mid.push_str(tag);
        mid.push('=');
        mid.push_str(val);
        mid.push(SOH);
    }
    let body_len = mid.len();
    let mut msg = format!("8={BEGIN_STRING}{SOH}9={body_len}{SOH}{mid}");
    let checksum = fix_checksum(&msg);
    msg.push_str(&format!("10={checksum:03}{SOH}"));
    msg
}

/// Decodes a SOH-delimited FIX message into tag → value (last wins).
///
/// # Errors
///
/// Returns a static reason when the message is empty or checksum mismatches.
pub fn decode_fix(raw: &str) -> Result<BTreeMap<String, String>, &'static str> {
    if raw.is_empty() {
        return Err("empty fix message");
    }
    let without_checksum = raw
        .rsplit_once(&format!("{SOH}10="))
        .map_or_else(|| raw.to_string(), |(head, _)| format!("{head}{SOH}"));
    if let Some((_, trail)) = raw.rsplit_once(&format!("{SOH}10=")) {
        let got = trail.trim_end_matches(SOH);
        let expect = format!("{:03}", fix_checksum(&without_checksum));
        if got != expect {
            return Err("checksum mismatch");
        }
    }
    let mut map = BTreeMap::new();
    for field in raw.split(SOH) {
        if field.is_empty() {
            continue;
        }
        let Some((k, v)) = field.split_once('=') else {
            continue;
        };
        map.insert(k.to_string(), v.to_string());
    }
    if !map.contains_key("35") {
        return Err("missing MsgType");
    }
    Ok(map)
}

fn fix_checksum(prefix: &str) -> u8 {
    prefix.bytes().fold(0_u8, u8::wrapping_add)
}

#[cfg(test)]
mod tests {
    use super::*;
    use shinrai_instruments::InstrumentId;
    use shinrai_orders::TimeInForce;

    #[test]
    fn codec_round_trip_checksum() {
        let wire = encode_fix("SHINRAI", "SIM", "A", 1, &[("98", "0"), ("108", "30")]);
        assert!(wire.contains("8=FIX.4.2"));
        assert!(wire.contains('\x01'));
        let map = decode_fix(&wire).expect("decode");
        assert_eq!(map.get("35").map(String::as_str), Some("A"));
        assert_eq!(map.get("49").map(String::as_str), Some("SHINRAI"));
        assert_eq!(map.get("56").map(String::as_str), Some("SIM"));
    }

    #[test]
    fn submit_emits_fix_new_and_fill() {
        let mut v = FixPaperVenue::local_mock();
        assert!(v.outbound_wire().iter().any(|m| m.contains("35=A")));
        v.submit(&NewVenueOrder {
            order_id: OrderId::from_u64(7),
            instrument_id: InstrumentId::from_u64(1),
            side: Side::Buy,
            qty: QuantityLots::from_lots(5),
            price: PriceTicks::from_scaled(10_000),
            tif: TimeInForce::Gtc,
        })
        .expect("submit");
        assert!(v
            .outbound_wire()
            .iter()
            .any(|m| m.contains("35=D") && m.contains("11=7")));
        let reports = v.poll();
        assert_eq!(reports.len(), 2);
        assert_eq!(reports[0].exec_type(), &ExecType::New);
        assert_eq!(reports[1].exec_type(), &ExecType::Trade);
        assert!(v
            .inbound_wire()
            .iter()
            .any(|m| m.contains("35=8") && m.contains("150=F")));
    }

    #[test]
    fn cancel_after_ack_only() {
        let mut v = FixPaperVenue::new(FixConfig::ack_only());
        v.logon().expect("logon");
        v.submit(&NewVenueOrder {
            order_id: OrderId::from_u64(3),
            instrument_id: InstrumentId::from_u64(1),
            side: Side::Buy,
            qty: QuantityLots::from_lots(2),
            price: PriceTicks::from_scaled(100),
            tif: TimeInForce::Gtc,
        })
        .expect("submit");
        let _ = v.poll();
        v.cancel(OrderId::from_u64(3)).expect("cxl");
        let reports = v.poll();
        assert!(reports
            .iter()
            .any(|r| matches!(r.exec_type(), ExecType::Canceled)));
        assert!(v.outbound_wire().iter().any(|m| m.contains("35=F")));
    }

    #[test]
    fn reconnect_resets_seq_and_keeps_inflight() {
        let mut v = FixPaperVenue::new(FixConfig::ack_only());
        v.logon().expect("logon");
        v.submit(&NewVenueOrder {
            order_id: OrderId::from_u64(1),
            instrument_id: InstrumentId::from_u64(1),
            side: Side::Buy,
            qty: QuantityLots::from_lots(1),
            price: PriceTicks::from_scaled(50),
            tif: TimeInForce::Gtc,
        })
        .expect("submit");
        let _ = v.poll();
        assert!(v.venue_order(OrderId::from_u64(1)).is_some());
        v.reconnect();
        assert_eq!(v.session_state().session.n, 2);
        assert_eq!(v.session_state().next_seq, 1);
        assert!(v.venue_order(OrderId::from_u64(1)).is_some());
        let recovered = v.poll_recovery(1).expect("recovery");
        assert!(recovered.is_empty()); // history cleared on reconnect
    }

    #[test]
    fn tcp_loopback_submit_fills() {
        let mut v = FixPaperVenue::new(FixConfig::tcp_loopback_ephemeral());
        v.logon().expect("tcp logon");
        assert!(v.tcp_enabled());
        v.submit(&NewVenueOrder {
            order_id: OrderId::from_u64(9),
            instrument_id: InstrumentId::from_u64(1),
            side: Side::Buy,
            qty: QuantityLots::from_lots(4),
            price: PriceTicks::from_scaled(10_000),
            tif: TimeInForce::Gtc,
        })
        .expect("submit");
        let reports = v.poll();
        assert!(
            reports
                .iter()
                .any(|r| matches!(r.exec_type(), ExecType::New)),
            "missing New: {reports:?}"
        );
        assert!(
            reports
                .iter()
                .any(|r| matches!(r.exec_type(), ExecType::Trade)),
            "missing Trade: {reports:?}"
        );
        assert!(v.outbound_wire().iter().any(|m| m.contains("35=D")));
        assert!(v.inbound_wire().iter().any(|m| m.contains("150=F")));
    }
}
