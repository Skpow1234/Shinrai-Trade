//! REST paper venue: JSON over [`HttpTransport`] → [`ExecutionVenue`].
//!
//! Offline-safe by default via [`LocalPaperHttp`] (in-process broker that still
//! round-trips JSON). A live HTTP client can implement [`HttpTransport`] later.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use shinrai_instruments::{PriceTicks, QuantityLots};
use shinrai_orders::{ExecId, OrderId, Side, VenueOrderId};

use crate::error::ExecutionError;
use crate::report::{ExecType, ExecutionReport, SessionId};
use crate::session::VenueSessionState;
use crate::venue::{ExecutionVenue, NewVenueOrder, VenueOrderSnapshot};

/// HTTP method for the paper REST surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    /// GET.
    Get,
    /// POST.
    Post,
    /// DELETE.
    Delete,
}

/// One outbound HTTP-shaped request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequest {
    /// Method.
    pub method: HttpMethod,
    /// Path including leading `/`.
    pub path: String,
    /// UTF-8 JSON body (empty for DELETE).
    pub body: String,
}

/// One HTTP-shaped response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    /// Status code.
    pub status: u16,
    /// UTF-8 JSON body.
    pub body: String,
}

/// Sync transport used by [`RestPaperVenue`] (mock, local, or live client).
pub trait HttpTransport: Send {
    /// Performs one request.
    ///
    /// # Errors
    ///
    /// Returns disconnect / transport errors.
    fn request(&mut self, req: &HttpRequest) -> Result<HttpResponse, ExecutionError>;
}

/// Wire JSON for submit.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SubmitBody {
    order_id: u64,
    instrument_id: u64,
    side: String,
    qty: i64,
    price: i64,
}

/// Wire JSON for one report in a response batch.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ReportBody {
    order_id: u64,
    venue_order_id: String,
    exec_id: Option<String>,
    exec_type: String,
    reason: Option<String>,
    qty: i64,
    price: i64,
    session: u32,
    seq: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ReportsResponse {
    reports: Vec<ReportBody>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SessionBody {
    session: u32,
    next_seq: u64,
    connected: bool,
}

#[derive(Debug, Clone)]
struct Inflight {
    venue_order_id: VenueOrderId,
    order_qty: i64,
    cum_qty: i64,
    price: PriceTicks,
    canceled: bool,
}

/// In-process paper broker that speaks the REST JSON wire format.
#[derive(Debug)]
struct LocalPaperState {
    auto_fill: bool,
    connected: bool,
    next_venue: u64,
    next_exec: u64,
    next_seq: u64,
    session: u32,
    inflight: HashMap<OrderId, Inflight>,
    /// Current-session report journal (wire form) for recovery.
    history: Vec<ReportBody>,
}

impl LocalPaperState {
    fn handle(&mut self, req: &HttpRequest) -> Result<HttpResponse, ExecutionError> {
        match (req.method, req.path.as_str()) {
            (HttpMethod::Post, "/v1/orders") => self.handle_submit(&req.body),
            (HttpMethod::Delete, path) if path.starts_with("/v1/orders/") => {
                let id = path
                    .trim_start_matches("/v1/orders/")
                    .parse::<u64>()
                    .map_err(|_| ExecutionError::InvalidIdentifier)?;
                self.handle_cancel(OrderId::from_u64(id))
            }
            (HttpMethod::Get, "/v1/session") => self.handle_session(),
            (HttpMethod::Get, path) if path.starts_with("/v1/reports") => {
                self.handle_reports(path)
            }
            (HttpMethod::Post, "/v1/session/disconnect") => {
                self.connected = false;
                Ok(HttpResponse {
                    status: 200,
                    body: String::new(),
                })
            }
            (HttpMethod::Post, "/v1/session/reconnect") => {
                self.connected = true;
                self.session = self.session.saturating_add(1);
                self.next_seq = 1;
                self.history.clear();
                Ok(HttpResponse {
                    status: 200,
                    body: String::new(),
                })
            }
            _ => Ok(HttpResponse {
                status: 404,
                body: r#"{"error":"not_found"}"#.into(),
            }),
        }
    }

    fn handle_session(&self) -> Result<HttpResponse, ExecutionError> {
        let body = serde_json::to_string(&SessionBody {
            session: self.session,
            next_seq: self.next_seq,
            connected: self.connected,
        })
        .map_err(|_| ExecutionError::InvalidState("encode session"))?;
        Ok(HttpResponse { status: 200, body })
    }

    fn handle_reports(&self, path: &str) -> Result<HttpResponse, ExecutionError> {
        if !self.connected {
            return Ok(HttpResponse {
                status: 503,
                body: r#"{"error":"disconnected"}"#.into(),
            });
        }
        let from_seq = parse_from_seq(path).unwrap_or(0);
        let reports: Vec<ReportBody> = self
            .history
            .iter()
            .filter(|r| r.session == self.session && r.seq >= from_seq)
            .cloned()
            .collect();
        let body = serde_json::to_string(&ReportsResponse { reports })
            .map_err(|_| ExecutionError::InvalidState("encode reports"))?;
        Ok(HttpResponse { status: 200, body })
    }

    fn handle_submit(&mut self, body: &str) -> Result<HttpResponse, ExecutionError> {
        if !self.connected {
            return Ok(HttpResponse {
                status: 503,
                body: r#"{"error":"disconnected"}"#.into(),
            });
        }
        let submit: SubmitBody = serde_json::from_str(body)
            .map_err(|_| ExecutionError::InvalidState("bad submit json"))?;
        if submit.qty <= 0 || submit.price <= 0 {
            return Ok(HttpResponse {
                status: 400,
                body: r#"{"error":"invalid_qty_or_price"}"#.into(),
            });
        }
        let order_id = OrderId::from_u64(submit.order_id);
        let venue_order_id = VenueOrderId::new(format!("REST-{}", self.next_venue))
            .map_err(|_| ExecutionError::InvalidIdentifier)?;
        self.next_venue = self.next_venue.saturating_add(1);
        let price = PriceTicks::from_scaled(submit.price);
        let qty = QuantityLots::from_lots(submit.qty);
        self.inflight.insert(
            order_id,
            Inflight {
                venue_order_id: venue_order_id.clone(),
                order_qty: submit.qty,
                cum_qty: 0,
                price,
                canceled: false,
            },
        );
        let mut reports = vec![self.report(
            order_id,
            &venue_order_id,
            None,
            "New",
            None,
            qty,
            price,
        )];
        if self.auto_fill {
            let exec_id = ExecId::new(format!("REST-E{}", self.next_exec))
                .map_err(|_| ExecutionError::InvalidIdentifier)?;
            self.next_exec = self.next_exec.saturating_add(1);
            if let Some(row) = self.inflight.get_mut(&order_id) {
                row.cum_qty = submit.qty;
            }
            reports.push(self.report(
                order_id,
                &venue_order_id,
                Some(exec_id.as_str().to_owned()),
                "Trade",
                None,
                qty,
                price,
            ));
        }
        let body = serde_json::to_string(&ReportsResponse { reports })
            .map_err(|_| ExecutionError::InvalidState("encode reports"))?;
        Ok(HttpResponse { status: 200, body })
    }

    fn handle_cancel(&mut self, order_id: OrderId) -> Result<HttpResponse, ExecutionError> {
        if !self.connected {
            return Ok(HttpResponse {
                status: 503,
                body: r#"{"error":"disconnected"}"#.into(),
            });
        }
        let Some(row) = self.inflight.get_mut(&order_id) else {
            return Ok(HttpResponse {
                status: 404,
                body: r#"{"error":"unknown_order"}"#.into(),
            });
        };
        if row.canceled {
            return Ok(HttpResponse {
                status: 409,
                body: r#"{"error":"already_canceled"}"#.into(),
            });
        }
        if row.cum_qty >= row.order_qty {
            return Ok(HttpResponse {
                status: 409,
                body: r#"{"error":"already_filled"}"#.into(),
            });
        }
        row.canceled = true;
        let venue_order_id = row.venue_order_id.clone();
        let leaves = QuantityLots::from_lots(row.order_qty - row.cum_qty);
        let price = row.price;
        let reports = vec![self.report(
            order_id,
            &venue_order_id,
            None,
            "Canceled",
            None,
            leaves,
            price,
        )];
        let body = serde_json::to_string(&ReportsResponse { reports })
            .map_err(|_| ExecutionError::InvalidState("encode reports"))?;
        Ok(HttpResponse { status: 200, body })
    }

    #[allow(clippy::too_many_arguments)]
    fn report(
        &mut self,
        order_id: OrderId,
        venue_order_id: &VenueOrderId,
        exec_id: Option<String>,
        exec_type: &str,
        reason: Option<String>,
        qty: QuantityLots,
        price: PriceTicks,
    ) -> ReportBody {
        let seq = self.next_seq;
        self.next_seq = self.next_seq.saturating_add(1);
        let body = ReportBody {
            order_id: order_id.get(),
            venue_order_id: venue_order_id.as_str().to_owned(),
            exec_id,
            exec_type: exec_type.to_owned(),
            reason,
            qty: qty.lots(),
            price: price.scaled(),
            session: self.session,
            seq,
        };
        self.history.push(body.clone());
        body
    }
}

fn parse_from_seq(path: &str) -> Option<u64> {
    let query = path.split_once('?')?.1;
    for part in query.split('&') {
        if let Some((k, v)) = part.split_once('=') {
            if k == "from_seq" {
                return v.parse().ok();
            }
        }
    }
    None
}

/// Default transport: local paper broker with JSON request/response.
#[derive(Debug, Clone)]
pub struct LocalPaperHttp {
    state: Arc<Mutex<LocalPaperState>>,
}

impl LocalPaperHttp {
    /// Happy-path auto-fill broker.
    #[must_use]
    pub fn happy_path() -> Self {
        Self {
            state: Arc::new(Mutex::new(LocalPaperState {
                auto_fill: true,
                connected: true,
                next_venue: 1,
                next_exec: 1,
                next_seq: 1,
                session: 1,
                inflight: HashMap::new(),
                history: Vec::new(),
            })),
        }
    }

    /// Ack-only (no auto fill).
    #[must_use]
    pub fn ack_only() -> Self {
        Self {
            state: Arc::new(Mutex::new(LocalPaperState {
                auto_fill: false,
                connected: true,
                next_venue: 1,
                next_exec: 1,
                next_seq: 1,
                session: 1,
                inflight: HashMap::new(),
                history: Vec::new(),
            })),
        }
    }
}

impl HttpTransport for LocalPaperHttp {
    fn request(&mut self, req: &HttpRequest) -> Result<HttpResponse, ExecutionError> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .handle(req)
    }
}

/// REST-shaped execution venue (queues reports from HTTP responses for [`poll`](ExecutionVenue::poll)).
#[derive(Debug, Clone)]
pub struct RestPaperVenue {
    transport: LocalPaperHttp,
    outbox: VecDeque<ExecutionReport>,
    /// Local mirror for `venue_order` snapshots (updated from successful responses).
    inflight: HashMap<OrderId, Inflight>,
}

impl RestPaperVenue {
    /// Creates a venue over the local JSON paper broker (auto-fill).
    #[must_use]
    pub fn local_happy_path() -> Self {
        Self {
            transport: LocalPaperHttp::happy_path(),
            outbox: VecDeque::new(),
            inflight: HashMap::new(),
        }
    }

    /// Creates a venue over the local JSON paper broker (ack only).
    #[must_use]
    pub fn local_ack_only() -> Self {
        Self {
            transport: LocalPaperHttp::ack_only(),
            outbox: VecDeque::new(),
            inflight: HashMap::new(),
        }
    }

    /// Restores a working order without HTTP (startup hydrate).
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
            VenueOrderId::new(format!("REST-H-{}", order_id.get()))
                .map_err(|_| ExecutionError::InvalidIdentifier)?
        };
        let row = Inflight {
            venue_order_id,
            order_qty: order_qty.lots(),
            cum_qty,
            price,
            canceled: false,
        };
        self.transport
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .inflight
            .insert(order_id, row.clone());
        self.inflight.insert(order_id, row);
        Ok(())
    }

    fn map_status(status: u16) -> Result<(), ExecutionError> {
        if status == 503 {
            return Err(ExecutionError::Disconnected);
        }
        Ok(())
    }

    fn enqueue_response(&mut self, resp: &HttpResponse) -> Result<(), ExecutionError> {
        Self::map_status(resp.status)?;
        if resp.status == 404 {
            return Err(ExecutionError::UnknownOrder {
                id: OrderId::from_u64(0),
            });
        }
        if resp.status >= 400 {
            return Err(ExecutionError::InvalidState("rest broker rejected"));
        }
        let parsed: ReportsResponse = serde_json::from_str(&resp.body)
            .map_err(|_| ExecutionError::InvalidState("bad reports json"))?;
        for raw in parsed.reports {
            let report = decode_report(raw)?;
            self.apply_inflight_from_report(&report);
            self.outbox.push_back(report);
        }
        Ok(())
    }

    fn apply_inflight_from_report(&mut self, report: &ExecutionReport) {
        match report.exec_type() {
            ExecType::New => {
                self.inflight.insert(
                    report.order_id(),
                    Inflight {
                        venue_order_id: report.venue_order_id().clone(),
                        order_qty: report.qty().lots(),
                        cum_qty: 0,
                        price: report.price(),
                        canceled: false,
                    },
                );
            }
            ExecType::Trade => {
                if let Some(row) = self.inflight.get_mut(&report.order_id()) {
                    row.cum_qty = row
                        .cum_qty
                        .saturating_add(report.qty().lots())
                        .min(row.order_qty);
                }
            }
            ExecType::Canceled => {
                if let Some(row) = self.inflight.get_mut(&report.order_id()) {
                    row.canceled = true;
                }
            }
            _ => {}
        }
    }
}

fn decode_report(raw: ReportBody) -> Result<ExecutionReport, ExecutionError> {
    let venue_order_id =
        VenueOrderId::new(raw.venue_order_id).map_err(|_| ExecutionError::InvalidIdentifier)?;
    let exec_id = match raw.exec_id {
        Some(s) => Some(ExecId::new(s).map_err(|_| ExecutionError::InvalidIdentifier)?),
        None => None,
    };
    let exec_type = match raw.exec_type.as_str() {
        "New" => ExecType::New,
        "Trade" => ExecType::Trade,
        "Canceled" => ExecType::Canceled,
        "Rejected" => ExecType::Rejected {
            reason: raw.reason.unwrap_or_else(|| "rejected".into()),
        },
        "Expired" => ExecType::Expired,
        "Replaced" => ExecType::Replaced,
        "CancelReject" => ExecType::CancelReject {
            reason: raw.reason.unwrap_or_else(|| "cancel_reject".into()),
        },
        _ => return Err(ExecutionError::InvalidState("unknown exec_type")),
    };
    Ok(ExecutionReport::new(
        OrderId::from_u64(raw.order_id),
        venue_order_id,
        exec_id,
        exec_type,
        QuantityLots::from_lots(raw.qty),
        PriceTicks::from_scaled(raw.price),
        SessionId::new(raw.session),
        raw.seq,
    ))
}

impl ExecutionVenue for RestPaperVenue {
    fn submit(&mut self, order: &NewVenueOrder) -> Result<(), ExecutionError> {
        if order.qty.lots() <= 0 || order.price.scaled() <= 0 {
            return Err(ExecutionError::InvalidQuantity);
        }
        let side = match order.side {
            Side::Buy => "Buy",
            Side::Sell => "Sell",
        };
        let body = serde_json::to_string(&SubmitBody {
            order_id: order.order_id.get(),
            instrument_id: order.instrument_id.get(),
            side: side.into(),
            qty: order.qty.lots(),
            price: order.price.scaled(),
        })
        .map_err(|_| ExecutionError::InvalidState("encode submit"))?;
        let resp = self.transport.request(&HttpRequest {
            method: HttpMethod::Post,
            path: "/v1/orders".into(),
            body,
        })?;
        self.enqueue_response(&resp)
    }

    fn cancel(&mut self, order_id: OrderId) -> Result<(), ExecutionError> {
        let resp = self.transport.request(&HttpRequest {
            method: HttpMethod::Delete,
            path: format!("/v1/orders/{}", order_id.get()),
            body: String::new(),
        })?;
        if resp.status == 503 {
            return Err(ExecutionError::Disconnected);
        }
        if resp.status == 404 {
            return Err(ExecutionError::UnknownOrder { id: order_id });
        }
        if resp.status == 409 {
            return Err(ExecutionError::InvalidState("cancel conflict"));
        }
        self.enqueue_response(&resp)
    }

    fn poll(&mut self) -> Vec<ExecutionReport> {
        self.outbox.drain(..).collect()
    }

    fn tick(&mut self, _ticks: u64) {}

    fn venue_order(&self, order_id: OrderId) -> Option<VenueOrderSnapshot> {
        self.inflight.get(&order_id).map(|row| VenueOrderSnapshot {
            order_id,
            order_qty: row.order_qty,
            cum_qty: row.cum_qty,
            canceled: row.canceled,
        })
    }

    fn venue_orders(&self) -> Vec<VenueOrderSnapshot> {
        self.inflight
            .iter()
            .map(|(id, row)| VenueOrderSnapshot {
                order_id: *id,
                order_qty: row.order_qty,
                cum_qty: row.cum_qty,
                canceled: row.canceled,
            })
            .collect()
    }

    fn session_state(&self) -> VenueSessionState {
        let mut transport = self.transport.clone();
        let Ok(resp) = transport.request(&HttpRequest {
            method: HttpMethod::Get,
            path: "/v1/session".into(),
            body: String::new(),
        }) else {
            return VenueSessionState::disconnected(SessionId::new(0), 0);
        };
        let Ok(body) = serde_json::from_str::<SessionBody>(&resp.body) else {
            return VenueSessionState::disconnected(SessionId::new(0), 0);
        };
        if body.connected {
            VenueSessionState::connected(SessionId::new(body.session), body.next_seq)
        } else {
            VenueSessionState::disconnected(SessionId::new(body.session), body.next_seq)
        }
    }

    fn disconnect(&mut self) {
        let _ = self.transport.request(&HttpRequest {
            method: HttpMethod::Post,
            path: "/v1/session/disconnect".into(),
            body: String::new(),
        });
    }

    fn reconnect(&mut self) {
        let _ = self.transport.request(&HttpRequest {
            method: HttpMethod::Post,
            path: "/v1/session/reconnect".into(),
            body: String::new(),
        });
        self.outbox.clear();
    }

    fn poll_recovery(&mut self, from_seq: u64) -> Result<Vec<ExecutionReport>, ExecutionError> {
        let resp = self.transport.request(&HttpRequest {
            method: HttpMethod::Get,
            path: format!("/v1/reports?from_seq={from_seq}"),
            body: String::new(),
        })?;
        Self::map_status(resp.status)?;
        if resp.status >= 400 {
            return Err(ExecutionError::InvalidState("rest broker rejected"));
        }
        let parsed: ReportsResponse = serde_json::from_str(&resp.body)
            .map_err(|_| ExecutionError::InvalidState("bad reports json"))?;
        parsed.reports.into_iter().map(decode_report).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shinrai_instruments::InstrumentId;

    #[test]
    fn rest_auto_fill_round_trips_json() {
        let mut venue = RestPaperVenue::local_happy_path();
        venue
            .submit(&NewVenueOrder {
                order_id: OrderId::from_u64(7),
                instrument_id: InstrumentId::from_u64(1),
                side: Side::Buy,
                qty: QuantityLots::from_lots(3),
                price: PriceTicks::from_scaled(10_000),
            })
            .expect("submit");
        let reports = venue.poll();
        assert_eq!(reports.len(), 2);
        assert!(matches!(reports[0].exec_type(), ExecType::New));
        assert!(matches!(reports[1].exec_type(), ExecType::Trade));
        assert!(reports[0].venue_order_id().as_str().starts_with("REST-"));
        assert_eq!(venue.venue_order(OrderId::from_u64(7)).unwrap().cum_qty, 3);
    }

    #[test]
    fn rest_cancel_after_ack_only() {
        let mut venue = RestPaperVenue::local_ack_only();
        venue
            .submit(&NewVenueOrder {
                order_id: OrderId::from_u64(2),
                instrument_id: InstrumentId::from_u64(1),
                side: Side::Buy,
                qty: QuantityLots::from_lots(2),
                price: PriceTicks::from_scaled(50),
            })
            .expect("submit");
        let _ = venue.poll();
        venue.cancel(OrderId::from_u64(2)).expect("cancel");
        let reports = venue.poll();
        assert_eq!(reports.len(), 1);
        assert!(matches!(reports[0].exec_type(), ExecType::Canceled));
    }

    #[test]
    fn disconnect_rejects_submit() {
        let mut venue = RestPaperVenue::local_ack_only();
        venue
            .submit(&NewVenueOrder {
                order_id: OrderId::from_u64(1),
                instrument_id: InstrumentId::from_u64(1),
                side: Side::Buy,
                qty: QuantityLots::from_lots(1),
                price: PriceTicks::from_scaled(10),
            })
            .expect("submit");
        let _ = venue.poll();
        venue.disconnect();
        assert!(!venue.session_state().connected);
        assert_eq!(
            venue.submit(&NewVenueOrder {
                order_id: OrderId::from_u64(2),
                instrument_id: InstrumentId::from_u64(1),
                side: Side::Buy,
                qty: QuantityLots::from_lots(1),
                price: PriceTicks::from_scaled(10),
            }),
            Err(ExecutionError::Disconnected)
        );
        assert_eq!(venue.poll_recovery(1), Err(ExecutionError::Disconnected));
        venue.reconnect();
        assert_eq!(venue.session_state().session.n, 2);
        assert_eq!(venue.session_state().next_seq, 1);
        assert!(venue.session_state().connected);
    }

    #[test]
    fn poll_recovery_after_happy_path() {
        let mut venue = RestPaperVenue::local_happy_path();
        venue
            .submit(&NewVenueOrder {
                order_id: OrderId::from_u64(9),
                instrument_id: InstrumentId::from_u64(1),
                side: Side::Buy,
                qty: QuantityLots::from_lots(2),
                price: PriceTicks::from_scaled(5),
            })
            .expect("submit");
        let _ = venue.poll();
        let gap = venue.poll_recovery(2).expect("recovery");
        assert_eq!(gap.len(), 1);
        assert_eq!(gap[0].seq(), 2);
        assert!(matches!(gap[0].exec_type(), ExecType::Trade));
    }
}
