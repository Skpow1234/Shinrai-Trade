//! FIX 4.2 subset venue ([`ExecutionVenue`]) over an in-process loopback.
//!
//! Encodes real SOH-delimited FIX for Logon / Heartbeat / `NewOrderSingle` /
//! Cancel / Replace and parses [`ExecutionReport`] (`35=8`) replies from a local
//! acceptor backed by the same auto-fill paper model as the sandbox. Not a
//! full FIX engine — trains the OMS on wire-shaped session traffic before a
//! vendor TCP adapter is wired.

use std::collections::{BTreeMap, HashMap, VecDeque};

use shinrai_instruments::{PriceTicks, QuantityLots};
use shinrai_orders::{ExecId, OrderId, Side, TimeInForce, VenueOrderId};

use crate::error::ExecutionError;
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
}

impl Default for FixConfig {
    fn default() -> Self {
        Self::local_mock()
    }
}

impl FixConfig {
    /// Local loopback happy-path (auto-fill).
    #[must_use]
    pub fn local_mock() -> Self {
        Self {
            sender_comp_id: "SHINRAI".into(),
            target_comp_id: "SIM".into(),
            auto_fill: true,
            heartbeat_interval_ticks: 5,
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

    /// Loads `CompID`s / heartbeat from the environment (local loopback; no TCP yet).
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

/// Wire-shaped FIX paper venue (in-process acceptor).
#[derive(Debug, Clone)]
pub struct FixPaperVenue {
    config: FixConfig,
    connected: bool,
    session: SessionId,
    /// Initiator outbound msg seq (tag 34).
    next_wire_out: u64,
    /// Acceptor outbound msg seq (tag 34 on wire).
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
    /// Encoded FIX messages produced by the local acceptor.
    inbound_wire: Vec<String>,
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
        }
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
    /// Returns [`ExecutionError::InvalidState`] when already connected.
    pub fn logon(&mut self) -> Result<u64, ExecutionError> {
        if self.connected {
            return Err(ExecutionError::InvalidState("already logged on"));
        }
        self.connected = true;
        let seq = self.emit_initiator("A", &[("98", "0"), ("108", "30")]);
        // Acceptor Logon ack (not an ExecutionReport).
        let _ = self.emit_acceptor("A", &[("98", "0"), ("108", "30")]);
        self.last_hb_tick = self.clock;
        Ok(seq)
    }

    /// Graceful logout (`35=5`).
    pub fn logout(&mut self) {
        if self.connected {
            let _ = self.emit_initiator("5", &[]);
            let _ = self.emit_acceptor("5", &[]);
        }
        self.connected = false;
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

    fn emit_initiator(&mut self, msg_type: &str, body: &[(&str, &str)]) -> u64 {
        let seq = self.next_wire_out;
        self.next_wire_out = self.next_wire_out.saturating_add(1);
        let wire = encode_fix(
            &self.config.sender_comp_id,
            &self.config.target_comp_id,
            msg_type,
            seq,
            body,
        );
        self.outbound_wire.push(wire);
        seq
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
        let _ = self.emit_initiator(
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
        );

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
        let _ = self.emit_initiator(
            "F",
            &[
                ("11", &cxl_id),
                ("41", &cl_ord),
                ("37", &venue),
            ],
        );

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
        let _ = self.emit_initiator(
            "G",
            &[
                ("11", &rpl_id),
                ("41", &cl_ord),
                ("37", &venue),
                ("38", &qty_s),
                ("44", &px_s),
                ("40", "2"),
            ],
        );
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
        self.outbox.drain(..).collect()
    }

    fn tick(&mut self, ticks: u64) {
        self.clock = self.clock.saturating_add(ticks);
        if !self.connected || self.config.heartbeat_interval_ticks == 0 {
            return;
        }
        if self
            .clock
            .saturating_sub(self.last_hb_tick)
            >= self.config.heartbeat_interval_ticks
        {
            let _ = self.emit_initiator("0", &[]);
            let _ = self.emit_acceptor("0", &[]);
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
    }

    fn reconnect(&mut self) {
        self.connected = true;
        self.session = SessionId::new(self.session.n.saturating_add(1));
        self.next_wire_out = 1;
        self.next_wire_in = 1;
        self.next_report_seq = 1;
        self.history.clear();
        self.outbox.clear();
        self.outbound_wire.clear();
        self.inbound_wire.clear();
        let _ = self.emit_initiator("A", &[("98", "0"), ("108", "30")]);
        let _ = self.emit_acceptor("A", &[("98", "0"), ("108", "30")]);
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
        assert!(v
            .outbound_wire()
            .iter()
            .any(|m| m.contains("35=A")));
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
        assert!(reports.iter().any(|r| matches!(r.exec_type(), ExecType::Canceled)));
        assert!(v
            .outbound_wire()
            .iter()
            .any(|m| m.contains("35=F")));
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
}
