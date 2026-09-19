//! Alpaca paper Trading API adapter (`ExecutionVenue`).
//!
//! Speaks Alpaca `/v2/orders` over [`HttpTransport`]-shaped clients. Local mock
//! needs no network; remote uses `APCA-API-KEY-ID` / `APCA-API-SECRET-KEY`.

use std::collections::{HashMap, VecDeque};

use serde::Deserialize;
use serde_json::json;
use shinrai_instruments::{InstrumentId, PriceTicks, QuantityLots};
use shinrai_orders::{ExecId, OrderId, Side, TimeInForce, VenueOrderId};

use crate::error::ExecutionError;
use crate::report::{ExecType, ExecutionReport, SessionId};
use crate::rest::{HttpMethod, HttpRequest, HttpResponse, HttpTransport};
use crate::session::VenueSessionState;
use crate::venue::{ExecutionVenue, NewVenueOrder, VenueOrderSnapshot, VenueTradeSnapshot};

/// Default Alpaca paper API root.
pub const ALPACA_PAPER_BASE_URL: &str = "https://paper-api.alpaca.markets";

/// Broker statement pulled from Alpaca for EOD reconciliation.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AlpacaBrokerStatement {
    /// Account cash snapshot.
    pub account: AlpacaAccountSnap,
    /// Open positions.
    pub positions: Vec<AlpacaPositionSnap>,
    /// FILL activities.
    pub fills: Vec<AlpacaFillSnap>,
}

/// Alpaca account cash (USD minor units / cents).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AlpacaAccountSnap {
    /// Currency code (typically `USD`).
    pub currency: String,
    /// Cash in minor units.
    pub cash_minor: i128,
}

/// Alpaca position row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlpacaPositionSnap {
    /// Ticker symbol.
    pub symbol: String,
    /// Signed lots (short = negative).
    pub qty: i64,
}

/// Alpaca FILL activity row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlpacaFillSnap {
    /// Venue activity / exec id.
    pub exec_id: String,
    /// Client order id when present (maps to internal order id).
    pub client_order_id: Option<String>,
    /// Symbol.
    pub symbol: String,
    /// Fill qty.
    pub qty: i64,
    /// Fill price in scale-2 ticks.
    pub price_ticks: i64,
}

/// Alpaca paper credentials + base URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlpacaConfig {
    /// `APCA-API-KEY-ID`.
    pub api_key: String,
    /// `APCA-API-SECRET-KEY`.
    pub api_secret: String,
    /// API root (no trailing slash).
    pub base_url: String,
    /// When true (local mock), submit auto-fills.
    pub auto_fill: bool,
}

impl AlpacaConfig {
    /// Paper defaults with empty credentials (local mock only).
    #[must_use]
    pub fn paper_mock() -> Self {
        Self {
            api_key: String::new(),
            api_secret: String::new(),
            base_url: ALPACA_PAPER_BASE_URL.to_owned(),
            auto_fill: true,
        }
    }

    /// Reads `SHINRAI_OG_ALPACA_*` (falls back to `APCA_API_*`).
    #[must_use]
    pub fn from_env() -> Option<Self> {
        let api_key = std::env::var("SHINRAI_OG_ALPACA_KEY")
            .or_else(|_| std::env::var("APCA_API_KEY_ID"))
            .ok()
            .filter(|s| !s.is_empty())?;
        let api_secret = std::env::var("SHINRAI_OG_ALPACA_SECRET")
            .or_else(|_| std::env::var("APCA_API_SECRET_KEY"))
            .ok()
            .filter(|s| !s.is_empty())?;
        let base_url = std::env::var("SHINRAI_OG_ALPACA_BASE_URL")
            .or_else(|_| std::env::var("APCA_API_BASE_URL"))
            .unwrap_or_else(|_| ALPACA_PAPER_BASE_URL.to_owned());
        Some(Self {
            api_key,
            api_secret,
            base_url: base_url.trim_end_matches('/').to_owned(),
            auto_fill: false,
        })
    }
}

#[derive(Debug, Clone)]
struct Inflight {
    venue_order_id: VenueOrderId,
    order_qty: i64,
    cum_qty: i64,
    /// Last fill qty already reported as Trade (async GET polling).
    reported_cum: i64,
    price: PriceTicks,
    canceled: bool,
}

/// Alpaca paper venue.
#[derive(Debug, Clone)]
pub struct AlpacaPaperVenue {
    config: AlpacaConfig,
    transport: AlpacaTransport,
    connected: bool,
    session: SessionId,
    next_seq: u64,
    next_exec: u64,
    /// `InstrumentId` → ticker (injected by OG / tests).
    symbols: HashMap<InstrumentId, String>,
    inflight: HashMap<OrderId, Inflight>,
    outbox: VecDeque<ExecutionReport>,
    history: Vec<ExecutionReport>,
}

#[derive(Debug, Clone)]
enum AlpacaTransport {
    Local(LocalAlpacaHttp),
    Remote(RemoteAlpacaHttp),
}

impl HttpTransport for AlpacaTransport {
    fn request(&mut self, req: &HttpRequest) -> Result<HttpResponse, ExecutionError> {
        match self {
            Self::Local(t) => t.request(req),
            Self::Remote(t) => t.request(req),
        }
    }
}

impl AlpacaPaperVenue {
    /// Local mock with auto-fill (no network).
    #[must_use]
    pub fn local_mock(symbols: HashMap<InstrumentId, String>) -> Self {
        Self {
            config: AlpacaConfig::paper_mock(),
            transport: AlpacaTransport::Local(LocalAlpacaHttp::happy_path()),
            connected: true,
            session: SessionId::new(1),
            next_seq: 1,
            next_exec: 1,
            symbols,
            inflight: HashMap::new(),
            outbox: VecDeque::new(),
            history: Vec::new(),
        }
    }

    /// Remote Alpaca paper API.
    ///
    /// # Errors
    ///
    /// Returns transport errors when the HTTP client cannot be built.
    pub fn remote(
        config: AlpacaConfig,
        symbols: HashMap<InstrumentId, String>,
    ) -> Result<Self, ExecutionError> {
        let transport = AlpacaTransport::Remote(RemoteAlpacaHttp::new(&config)?);
        Ok(Self {
            config,
            transport,
            connected: true,
            session: SessionId::new(1),
            next_seq: 1,
            next_exec: 1,
            symbols,
            inflight: HashMap::new(),
            outbox: VecDeque::new(),
            history: Vec::new(),
        })
    }

    /// Restores working order without HTTP (hydrate).
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
        let venue_order_id = venue_order_id.unwrap_or_else(|| {
            VenueOrderId::new(format!("ALPACA-HYDRATE-{}", order_id.get())).expect("vid")
        });
        self.inflight.insert(
            order_id,
            Inflight {
                venue_order_id,
                order_qty: order_qty.lots(),
                cum_qty,
                reported_cum: cum_qty,
                price,
                canceled: false,
            },
        );
        Ok(())
    }

    /// Pulls Alpaca account / positions / FILL activities for EOD reconciliation.
    ///
    /// Cash is reported in USD minor units (cents). Fills include `client_order_id`
    /// when present so the gateway can map to internal order ids.
    ///
    /// # Errors
    ///
    /// Returns transport / parse errors.
    pub fn fetch_broker_statement(&mut self) -> Result<AlpacaBrokerStatement, ExecutionError> {
        let account = self.fetch_account()?;
        let positions = self.fetch_positions()?;
        let fills = self.fetch_fill_activities()?;
        Ok(AlpacaBrokerStatement {
            account,
            positions,
            fills,
        })
    }

    /// Reloads open Alpaca orders into the inflight map (reconnect recovery).
    ///
    /// # Errors
    ///
    /// Returns transport / parse errors.
    pub fn rehydrate_open_orders(&mut self) -> Result<(), ExecutionError> {
        let resp = self.transport.request(&HttpRequest {
            method: HttpMethod::Get,
            path: "/v2/orders?status=open".into(),
            body: String::new(),
        })?;
        if resp.status >= 400 {
            return Err(ExecutionError::Transport(format!(
                "alpaca open orders {}: {}",
                resp.status, resp.body
            )));
        }
        let rows: Vec<AlpacaOpenOrderJson> = serde_json::from_str(&resp.body)
            .map_err(|e| ExecutionError::Transport(e.to_string()))?;
        for row in rows {
            let Ok(order_id) = row
                .client_order_id
                .as_deref()
                .and_then(|s| s.parse::<u64>().ok())
                .map(OrderId::from_u64)
                .ok_or(())
            else {
                continue;
            };
            let Ok(venue_order_id) = VenueOrderId::new(row.id) else {
                continue;
            };
            let order_qty: i64 = row.qty.as_deref().and_then(|s| s.parse().ok()).unwrap_or(0);
            let cum_qty: i64 = row
                .filled_qty
                .as_deref()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            let price = row
                .limit_price
                .as_deref()
                .and_then(Self::parse_price_to_ticks)
                .unwrap_or_else(|| PriceTicks::from_scaled(1));
            if order_qty <= 0 {
                continue;
            }
            self.inflight.insert(
                order_id,
                Inflight {
                    venue_order_id,
                    order_qty,
                    cum_qty,
                    reported_cum: cum_qty,
                    price,
                    canceled: false,
                },
            );
        }
        Ok(())
    }

    fn fetch_account(&mut self) -> Result<AlpacaAccountSnap, ExecutionError> {
        let resp = self.transport.request(&HttpRequest {
            method: HttpMethod::Get,
            path: "/v2/account".into(),
            body: String::new(),
        })?;
        if resp.status >= 400 {
            return Err(ExecutionError::Transport(format!(
                "alpaca account {}: {}",
                resp.status, resp.body
            )));
        }
        let v: serde_json::Value = serde_json::from_str(&resp.body)
            .map_err(|e| ExecutionError::Transport(e.to_string()))?;
        let cash_raw = v.get("cash").and_then(|x| x.as_str()).unwrap_or("0");
        let cash_minor = parse_usd_to_cents(cash_raw).unwrap_or(0);
        Ok(AlpacaAccountSnap {
            currency: v
                .get("currency")
                .and_then(|x| x.as_str())
                .unwrap_or("USD")
                .to_owned(),
            cash_minor,
        })
    }

    fn fetch_positions(&mut self) -> Result<Vec<AlpacaPositionSnap>, ExecutionError> {
        let resp = self.transport.request(&HttpRequest {
            method: HttpMethod::Get,
            path: "/v2/positions".into(),
            body: String::new(),
        })?;
        if resp.status >= 400 {
            return Err(ExecutionError::Transport(format!(
                "alpaca positions {}: {}",
                resp.status, resp.body
            )));
        }
        let rows: Vec<AlpacaPositionJson> = serde_json::from_str(&resp.body)
            .map_err(|e| ExecutionError::Transport(e.to_string()))?;
        Ok(rows
            .into_iter()
            .filter_map(|r| {
                let qty: i64 = r.qty.parse().ok()?;
                let signed = if r.side.eq_ignore_ascii_case("short") {
                    -qty.abs()
                } else {
                    qty.abs()
                };
                Some(AlpacaPositionSnap {
                    symbol: r.symbol,
                    qty: signed,
                })
            })
            .collect())
    }

    fn fetch_fill_activities(&mut self) -> Result<Vec<AlpacaFillSnap>, ExecutionError> {
        let resp = self.transport.request(&HttpRequest {
            method: HttpMethod::Get,
            path: "/v2/account/activities/FILL".into(),
            body: String::new(),
        })?;
        if resp.status >= 400 {
            // Some paper accounts return empty; treat 404 as no fills.
            if resp.status == 404 {
                return Ok(Vec::new());
            }
            return Err(ExecutionError::Transport(format!(
                "alpaca activities {}: {}",
                resp.status, resp.body
            )));
        }
        let rows: Vec<AlpacaActivityJson> = serde_json::from_str(&resp.body)
            .map_err(|e| ExecutionError::Transport(e.to_string()))?;
        Ok(rows
            .into_iter()
            .filter_map(|r| {
                let qty: i64 = r.qty.as_deref()?.parse().ok()?;
                let price_ticks = r
                    .price
                    .as_deref()
                    .and_then(Self::parse_price_to_ticks)
                    .map(|p| p.scaled())
                    .unwrap_or(0);
                Some(AlpacaFillSnap {
                    exec_id: r.id,
                    client_order_id: r.client_order_id,
                    symbol: r.symbol.unwrap_or_default(),
                    qty,
                    price_ticks,
                })
            })
            .collect())
    }

    fn symbol_for(&self, id: InstrumentId) -> Result<&str, ExecutionError> {
        self.symbols
            .get(&id)
            .map(String::as_str)
            .ok_or_else(|| ExecutionError::Transport(format!("no symbol for instrument {id:?}")))
    }

    fn push_report(
        &mut self,
        order_id: OrderId,
        venue_order_id: VenueOrderId,
        exec_id: Option<ExecId>,
        exec_type: ExecType,
        qty: QuantityLots,
        price: PriceTicks,
    ) {
        let seq = self.next_seq;
        self.next_seq = self.next_seq.saturating_add(1);
        let report = ExecutionReport::new(
            order_id,
            venue_order_id,
            exec_id,
            exec_type,
            qty,
            price,
            self.session,
            seq,
        );
        self.history.push(report.clone());
        self.outbox.push_back(report);
    }

    fn alloc_exec(&mut self) -> Result<ExecId, ExecutionError> {
        let id = ExecId::new(format!("ALPACA-E{}", self.next_exec))
            .map_err(|_| ExecutionError::InvalidIdentifier)?;
        self.next_exec = self.next_exec.saturating_add(1);
        Ok(id)
    }

    fn ticks_to_limit_price(ticks: PriceTicks) -> String {
        // phase1 equities use scale-2 ticks (10000 → "100.00").
        let scaled = ticks.scaled();
        let whole = scaled / 100;
        let frac = scaled.rem_euclid(100);
        format!("{whole}.{frac:02}")
    }

    fn parse_price_to_ticks(raw: &str) -> Option<PriceTicks> {
        let raw = raw.trim();
        if raw.is_empty() {
            return None;
        }
        let (neg, s) = raw
            .strip_prefix('-')
            .map_or((false, raw), |rest| (true, rest));
        let mut parts = s.splitn(2, '.');
        let whole: i64 = parts.next()?.parse().ok()?;
        let frac_str = parts.next().unwrap_or("0");
        let mut frac_digits: String = frac_str.chars().take(2).collect();
        while frac_digits.len() < 2 {
            frac_digits.push('0');
        }
        let frac: i64 = frac_digits.parse().ok()?;
        let scaled = whole.saturating_mul(100).saturating_add(frac);
        let scaled = if neg { scaled.saturating_neg() } else { scaled };
        if scaled <= 0 {
            return None;
        }
        Some(PriceTicks::from_scaled(scaled))
    }

    /// GETs open orders and emits Trade reports for fill deltas (async paper fills).
    fn refresh_open_fills(&mut self) {
        let open: Vec<(OrderId, String, i64, i64, PriceTicks)> = self
            .inflight
            .iter()
            .filter(|(_, row)| !row.canceled && row.reported_cum < row.order_qty)
            .map(|(id, row)| {
                (
                    *id,
                    row.venue_order_id.as_str().to_owned(),
                    row.order_qty,
                    row.reported_cum,
                    row.price,
                )
            })
            .collect();
        for (order_id, venue_id, order_qty, reported_cum, fallback_px) in open {
            let Ok(resp) = self.transport.request(&HttpRequest {
                method: HttpMethod::Get,
                path: format!("/v2/orders/{venue_id}"),
                body: String::new(),
            }) else {
                continue;
            };
            if resp.status >= 400 {
                continue;
            }
            let Ok(parsed) = serde_json::from_str::<AlpacaOrderJson>(&resp.body) else {
                continue;
            };
            let filled: i64 = parsed
                .filled_qty
                .as_deref()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0)
                .clamp(0, order_qty);
            let status = parsed.status.to_ascii_lowercase();
            let done = matches!(
                status.as_str(),
                "filled" | "partially_filled" | "canceled" | "expired" | "rejected"
            ) || filled > reported_cum;
            if !done && filled <= reported_cum {
                continue;
            }
            let fill_px = parsed
                .filled_avg_price
                .as_deref()
                .and_then(Self::parse_price_to_ticks)
                .unwrap_or(fallback_px);
            if filled > reported_cum {
                let delta = filled - reported_cum;
                let Ok(exec_id) = self.alloc_exec() else {
                    continue;
                };
                let venue_order_id = VenueOrderId::new(venue_id.clone()).ok();
                let Some(venue_order_id) = venue_order_id else {
                    continue;
                };
                if let Some(row) = self.inflight.get_mut(&order_id) {
                    row.cum_qty = filled;
                    row.reported_cum = filled;
                    if status == "canceled" || status == "expired" {
                        row.canceled = true;
                    }
                }
                self.push_report(
                    order_id,
                    venue_order_id,
                    Some(exec_id),
                    ExecType::Trade,
                    QuantityLots::from_lots(delta),
                    fill_px,
                );
            } else if matches!(status.as_str(), "canceled" | "expired") {
                if let Some(row) = self.inflight.get_mut(&order_id) {
                    if !row.canceled {
                        row.canceled = true;
                        let leaves = QuantityLots::from_lots(row.order_qty - row.cum_qty);
                        let price = row.price;
                        let venue_order_id = row.venue_order_id.clone();
                        self.push_report(
                            order_id,
                            venue_order_id,
                            None,
                            ExecType::Canceled,
                            leaves,
                            price,
                        );
                    }
                }
            }
        }
    }
}

impl ExecutionVenue for AlpacaPaperVenue {
    fn submit(&mut self, order: &NewVenueOrder) -> Result<(), ExecutionError> {
        if !self.connected {
            return Err(ExecutionError::Disconnected);
        }
        if order.qty.lots() <= 0 || order.price.scaled() <= 0 {
            return Err(ExecutionError::InvalidQuantity);
        }
        let symbol = self.symbol_for(order.instrument_id)?.to_owned();
        let side = match order.side {
            Side::Buy => "buy",
            Side::Sell => "sell",
        };
        let tif = match order.tif {
            TimeInForce::Gtc => "gtc",
            TimeInForce::Ioc => "ioc",
            TimeInForce::Fok => "fok",
        };
        let body = json!({
            "symbol": symbol,
            "qty": order.qty.lots().to_string(),
            "side": side,
            "type": "limit",
            "time_in_force": tif,
            "limit_price": Self::ticks_to_limit_price(order.price),
            "client_order_id": order.order_id.get().to_string(),
        })
        .to_string();
        let resp = self.transport.request(&HttpRequest {
            method: HttpMethod::Post,
            path: "/v2/orders".into(),
            body,
        })?;
        if resp.status >= 400 {
            let reason = resp.body.clone();
            self.push_report(
                order.order_id,
                VenueOrderId::new("ALPACA-REJECT")
                    .map_err(|_| ExecutionError::InvalidIdentifier)?,
                None,
                ExecType::Rejected { reason },
                order.qty,
                order.price,
            );
            return Ok(());
        }
        let parsed: AlpacaOrderJson = serde_json::from_str(&resp.body)
            .map_err(|e| ExecutionError::Transport(e.to_string()))?;
        let venue_order_id =
            VenueOrderId::new(parsed.id.clone()).map_err(|_| ExecutionError::InvalidIdentifier)?;
        self.inflight.insert(
            order.order_id,
            Inflight {
                venue_order_id: venue_order_id.clone(),
                order_qty: order.qty.lots(),
                cum_qty: 0,
                reported_cum: 0,
                price: order.price,
                canceled: false,
            },
        );
        self.push_report(
            order.order_id,
            venue_order_id.clone(),
            None,
            ExecType::New,
            order.qty,
            order.price,
        );
        if self.config.auto_fill || matches!(parsed.status.as_str(), "filled") {
            let filled: i64 = parsed
                .filled_qty
                .as_deref()
                .and_then(|s| s.parse().ok())
                .unwrap_or(order.qty.lots());
            if filled > 0 {
                if let Some(row) = self.inflight.get_mut(&order.order_id) {
                    row.cum_qty = filled.min(row.order_qty);
                    row.reported_cum = row.cum_qty;
                }
                let exec_id = self.alloc_exec()?;
                self.push_report(
                    order.order_id,
                    venue_order_id,
                    Some(exec_id),
                    ExecType::Trade,
                    QuantityLots::from_lots(filled),
                    order.price,
                );
            }
        }
        Ok(())
    }

    fn cancel(&mut self, order_id: OrderId) -> Result<(), ExecutionError> {
        if !self.connected {
            return Err(ExecutionError::Disconnected);
        }
        let Some(row) = self.inflight.get(&order_id) else {
            return Err(ExecutionError::UnknownOrder { id: order_id });
        };
        if row.canceled {
            return Err(ExecutionError::InvalidState("already canceled"));
        }
        let venue_id = row.venue_order_id.as_str().to_owned();
        let resp = self.transport.request(&HttpRequest {
            method: HttpMethod::Delete,
            path: format!("/v2/orders/{venue_id}"),
            body: String::new(),
        })?;
        if resp.status >= 400 {
            return Err(ExecutionError::Transport(format!(
                "alpaca cancel {}: {}",
                resp.status, resp.body
            )));
        }
        let row = self.inflight.get_mut(&order_id).expect("inflight");
        row.canceled = true;
        let venue_order_id = row.venue_order_id.clone();
        let leaves = QuantityLots::from_lots(row.order_qty - row.cum_qty);
        let price = row.price;
        self.push_report(
            order_id,
            venue_order_id,
            None,
            ExecType::Canceled,
            leaves,
            price,
        );
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
        let Some(row) = self.inflight.get(&order_id) else {
            return Err(ExecutionError::UnknownOrder { id: order_id });
        };
        if row.canceled {
            return Err(ExecutionError::InvalidState("already canceled"));
        }
        if new_qty.lots() < row.cum_qty {
            return Err(ExecutionError::InvalidQuantity);
        }
        let venue_id = row.venue_order_id.as_str().to_owned();
        let body = json!({
            "qty": new_qty.lots().to_string(),
            "limit_price": Self::ticks_to_limit_price(new_price),
        })
        .to_string();
        let resp = self.transport.request(&HttpRequest {
            method: HttpMethod::Patch,
            path: format!("/v2/orders/{venue_id}"),
            body,
        })?;
        if resp.status >= 400 {
            return Err(ExecutionError::Transport(format!(
                "alpaca replace {}: {}",
                resp.status, resp.body
            )));
        }
        let row = self.inflight.get_mut(&order_id).expect("inflight");
        row.order_qty = new_qty.lots();
        row.price = new_price;
        let venue_order_id = row.venue_order_id.clone();
        self.push_report(
            order_id,
            venue_order_id,
            None,
            ExecType::Replaced,
            new_qty,
            new_price,
        );
        Ok(())
    }

    fn poll(&mut self) -> Vec<ExecutionReport> {
        if !self.outbox.is_empty() {
            return self.outbox.drain(..).collect();
        }
        if self.connected {
            self.refresh_open_fills();
        }
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

    fn trade_execs(&self) -> Vec<VenueTradeSnapshot> {
        self.history
            .iter()
            .filter_map(|r| {
                if !matches!(r.exec_type(), ExecType::Trade) {
                    return None;
                }
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
            VenueSessionState::connected(self.session, self.next_seq)
        } else {
            VenueSessionState::disconnected(self.session, self.next_seq)
        }
    }

    fn disconnect(&mut self) {
        self.connected = false;
    }

    fn reconnect(&mut self) {
        self.connected = true;
        self.session = SessionId::new(self.session.n.saturating_add(1));
        self.next_seq = 1;
        self.history.clear();
        self.outbox.clear();
        self.inflight.clear();
        if let Err(err) = self.rehydrate_open_orders() {
            eprintln!("shinrai-execution: alpaca reconnect rehydrate failed: {err}");
        }
    }

    fn poll_recovery(&mut self, from_seq: u64) -> Result<Vec<ExecutionReport>, ExecutionError> {
        if !self.connected {
            return Err(ExecutionError::Disconnected);
        }
        Ok(self
            .history
            .iter()
            .filter(|r| r.session() == self.session && r.seq() >= from_seq)
            .cloned()
            .collect())
    }
}

#[derive(Debug, Deserialize)]
struct AlpacaOrderJson {
    id: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    filled_qty: Option<String>,
    #[serde(default)]
    filled_avg_price: Option<String>,
    #[allow(dead_code)]
    #[serde(default)]
    limit_price: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AlpacaOpenOrderJson {
    id: String,
    #[serde(default)]
    client_order_id: Option<String>,
    #[serde(default)]
    qty: Option<String>,
    #[serde(default)]
    filled_qty: Option<String>,
    #[serde(default)]
    limit_price: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AlpacaPositionJson {
    symbol: String,
    qty: String,
    #[serde(default)]
    side: String,
}

#[derive(Debug, Deserialize)]
struct AlpacaActivityJson {
    id: String,
    #[serde(default)]
    qty: Option<String>,
    #[serde(default)]
    price: Option<String>,
    #[serde(default)]
    symbol: Option<String>,
    #[serde(default)]
    client_order_id: Option<String>,
}

/// In-process Alpaca-shaped mock (JSON round-trip, auto-fill).
#[derive(Debug, Clone)]
pub struct LocalAlpacaHttp {
    auto_fill: bool,
    /// When true and not yet filled, GET `/v2/orders/{id}` completes the fill.
    fills_on_get: bool,
    next_id: u64,
    orders: HashMap<String, LocalOrder>,
}

#[derive(Debug, Clone)]
struct LocalOrder {
    id: String,
    symbol: String,
    client_order_id: String,
    side: String,
    qty: i64,
    filled: i64,
    limit_price: String,
    canceled: bool,
}

impl LocalAlpacaHttp {
    /// Happy-path auto-fill mock.
    #[must_use]
    pub fn happy_path() -> Self {
        Self {
            auto_fill: true,
            fills_on_get: false,
            next_id: 1,
            orders: HashMap::new(),
        }
    }

    /// Ack-only submit; fills arrive on subsequent GET (async paper path).
    #[must_use]
    pub fn async_fill() -> Self {
        Self {
            auto_fill: false,
            fills_on_get: true,
            next_id: 1,
            orders: HashMap::new(),
        }
    }

    fn cash_minor_usd(&self) -> i128 {
        // Mock starting cash ($100,000.00) +/- filled notionals in USD cents.
        let mut cash: i128 = 10_000_000;
        for o in self.orders.values() {
            let cents = parse_usd_to_cents(&o.limit_price).unwrap_or(0);
            let notional = cents.saturating_mul(i128::from(o.filled.max(0)));
            if o.side == "buy" {
                cash = cash.saturating_sub(notional);
            } else {
                cash = cash.saturating_add(notional);
            }
        }
        cash
    }

    fn positions_json(&self) -> serde_json::Value {
        let mut by_symbol: HashMap<String, i64> = HashMap::new();
        for o in self.orders.values() {
            if o.filled <= 0 {
                continue;
            }
            let delta = if o.side == "buy" { o.filled } else { -o.filled };
            *by_symbol.entry(o.symbol.clone()).or_insert(0) += delta;
        }
        let arr: Vec<_> = by_symbol
            .into_iter()
            .filter(|(_, q)| *q != 0)
            .map(|(symbol, qty)| {
                let side = if qty >= 0 { "long" } else { "short" };
                json!({
                    "symbol": symbol,
                    "qty": qty.abs().to_string(),
                    "side": side,
                })
            })
            .collect();
        serde_json::Value::Array(arr)
    }

    fn fills_json(&self) -> serde_json::Value {
        let arr: Vec<_> = self
            .orders
            .values()
            .filter(|o| o.filled > 0)
            .map(|o| {
                json!({
                    "id": format!("fill-{}", o.id),
                    "activity_type": "FILL",
                    "symbol": o.symbol,
                    "qty": o.filled.to_string(),
                    "price": o.limit_price,
                    "side": o.side,
                    "order_id": o.id,
                    "client_order_id": o.client_order_id,
                })
            })
            .collect();
        serde_json::Value::Array(arr)
    }
}

fn parse_usd_to_cents(raw: &str) -> Option<i128> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let (neg, s) = raw
        .strip_prefix('-')
        .map_or((false, raw), |rest| (true, rest));
    let mut parts = s.splitn(2, '.');
    let whole: i128 = parts.next()?.parse().ok()?;
    let frac_str = parts.next().unwrap_or("0");
    let mut frac_digits: String = frac_str.chars().take(2).collect();
    while frac_digits.len() < 2 {
        frac_digits.push('0');
    }
    let frac: i128 = frac_digits.parse().ok()?;
    let cents = whole.saturating_mul(100).saturating_add(frac);
    Some(if neg { -cents } else { cents })
}

fn format_usd_from_cents(cents: i128) -> String {
    let neg = cents < 0;
    let abs = cents.unsigned_abs();
    let whole = abs / 100;
    let frac = abs % 100;
    if neg {
        format!("-{whole}.{frac:02}")
    } else {
        format!("{whole}.{frac:02}")
    }
}

impl HttpTransport for LocalAlpacaHttp {
    #[allow(clippy::too_many_lines)]
    fn request(&mut self, req: &HttpRequest) -> Result<HttpResponse, ExecutionError> {
        match (req.method, req.path.as_str()) {
            (HttpMethod::Post, "/v2/orders") => {
                let v: serde_json::Value = serde_json::from_str(&req.body)
                    .map_err(|e| ExecutionError::Transport(e.to_string()))?;
                let id = format!("alpaca-local-{}", self.next_id);
                self.next_id = self.next_id.saturating_add(1);
                let qty: i64 = v["qty"].as_str().and_then(|s| s.parse().ok()).unwrap_or(0);
                let limit_price = v["limit_price"].as_str().unwrap_or("0").to_owned();
                let symbol = v["symbol"].as_str().unwrap_or("UNKNOWN").to_owned();
                let side = v["side"].as_str().unwrap_or("buy").to_owned();
                let client_order_id = v["client_order_id"]
                    .as_str()
                    .map(ToOwned::to_owned)
                    .or_else(|| v["client_order_id"].as_u64().map(|n| n.to_string()))
                    .unwrap_or_default();
                let filled = if self.auto_fill { qty } else { 0 };
                let status = if self.auto_fill { "filled" } else { "accepted" };
                self.orders.insert(
                    id.clone(),
                    LocalOrder {
                        id: id.clone(),
                        symbol,
                        client_order_id: client_order_id.clone(),
                        side,
                        qty,
                        filled,
                        limit_price: limit_price.clone(),
                        canceled: false,
                    },
                );
                let body = json!({
                    "id": id,
                    "status": status,
                    "filled_qty": filled.to_string(),
                    "limit_price": limit_price,
                    "client_order_id": client_order_id,
                })
                .to_string();
                Ok(HttpResponse { status: 200, body })
            }
            (HttpMethod::Get, path) if path.starts_with("/v2/orders/") => {
                let id = path.trim_start_matches("/v2/orders/");
                if let Some(o) = self.orders.get_mut(id) {
                    if self.fills_on_get && !o.canceled && o.filled < o.qty {
                        o.filled = o.qty;
                    }
                    let status = if o.canceled {
                        "canceled"
                    } else if o.filled >= o.qty && o.qty > 0 {
                        "filled"
                    } else if o.filled > 0 {
                        "partially_filled"
                    } else {
                        "accepted"
                    };
                    Ok(HttpResponse {
                        status: 200,
                        body: json!({
                            "id": o.id,
                            "status": status,
                            "filled_qty": o.filled.to_string(),
                            "filled_avg_price": o.limit_price,
                            "limit_price": o.limit_price,
                        })
                        .to_string(),
                    })
                } else {
                    Ok(HttpResponse {
                        status: 404,
                        body: json!({"message":"not found"}).to_string(),
                    })
                }
            }
            (HttpMethod::Delete, path) if path.starts_with("/v2/orders/") => {
                let id = path.trim_start_matches("/v2/orders/");
                if let Some(o) = self.orders.get_mut(id) {
                    o.canceled = true;
                    Ok(HttpResponse {
                        status: 204,
                        body: String::new(),
                    })
                } else {
                    Ok(HttpResponse {
                        status: 404,
                        body: json!({"message":"not found"}).to_string(),
                    })
                }
            }
            (HttpMethod::Patch, path) if path.starts_with("/v2/orders/") => {
                let id = path.trim_start_matches("/v2/orders/");
                let v: serde_json::Value = serde_json::from_str(&req.body).unwrap_or_default();
                if let Some(o) = self.orders.get_mut(id) {
                    if let Some(q) = v["qty"].as_str().and_then(|s| s.parse().ok()) {
                        o.qty = q;
                    }
                    if let Some(p) = v["limit_price"].as_str() {
                        p.clone_into(&mut o.limit_price);
                    }
                    Ok(HttpResponse {
                        status: 200,
                        body: json!({
                            "id": o.id,
                            "status": "accepted",
                            "filled_qty": o.filled.to_string(),
                            "limit_price": o.limit_price,
                        })
                        .to_string(),
                    })
                } else {
                    Ok(HttpResponse {
                        status: 404,
                        body: json!({"message":"not found"}).to_string(),
                    })
                }
            }
            (HttpMethod::Post, path) if path.starts_with("/v2/orders/") => {
                // Back-compat: treat POST replace like PATCH.
                let id = path.trim_start_matches("/v2/orders/");
                let v: serde_json::Value = serde_json::from_str(&req.body).unwrap_or_default();
                if let Some(o) = self.orders.get_mut(id) {
                    if let Some(q) = v["qty"].as_str().and_then(|s| s.parse().ok()) {
                        o.qty = q;
                    }
                    if let Some(p) = v["limit_price"].as_str() {
                        p.clone_into(&mut o.limit_price);
                    }
                    Ok(HttpResponse {
                        status: 200,
                        body: json!({
                            "id": o.id,
                            "status": "accepted",
                            "filled_qty": o.filled.to_string(),
                            "limit_price": o.limit_price,
                        })
                        .to_string(),
                    })
                } else {
                    Ok(HttpResponse {
                        status: 404,
                        body: json!({"message":"not found"}).to_string(),
                    })
                }
            }
            (HttpMethod::Get, "/v2/account") => {
                let cash = format_usd_from_cents(self.cash_minor_usd());
                Ok(HttpResponse {
                    status: 200,
                    body: json!({
                        "cash": cash,
                        "equity": cash,
                        "buying_power": cash,
                        "currency": "USD",
                    })
                    .to_string(),
                })
            }
            (HttpMethod::Get, "/v2/positions") => {
                let positions = self.positions_json();
                Ok(HttpResponse {
                    status: 200,
                    body: positions.to_string(),
                })
            }
            (HttpMethod::Get, path) if path.starts_with("/v2/account/activities") => {
                let fills = self.fills_json();
                Ok(HttpResponse {
                    status: 200,
                    body: fills.to_string(),
                })
            }
            (HttpMethod::Get, path) if path == "/v2/orders" || path.starts_with("/v2/orders?") => {
                let list: Vec<_> = self
                    .orders
                    .values()
                    .filter(|o| !o.canceled && o.filled < o.qty)
                    .map(|o| {
                        json!({
                            "id": o.id,
                            "status": "accepted",
                            "filled_qty": o.filled.to_string(),
                            "qty": o.qty.to_string(),
                            "limit_price": o.limit_price,
                            "symbol": o.symbol,
                            "client_order_id": o.client_order_id,
                        })
                    })
                    .collect();
                Ok(HttpResponse {
                    status: 200,
                    body: serde_json::Value::Array(list).to_string(),
                })
            }
            _ => Ok(HttpResponse {
                status: 404,
                body: json!({"message":"unknown path"}).to_string(),
            }),
        }
    }
}

/// Blocking HTTP client for Alpaca paper/live Trading API.
#[derive(Debug, Clone)]
pub struct RemoteAlpacaHttp {
    base_url: String,
    api_key: String,
    api_secret: String,
    client: reqwest::blocking::Client,
}

impl RemoteAlpacaHttp {
    /// Builds a remote transport from config.
    ///
    /// # Errors
    ///
    /// Returns transport errors when the client cannot be built.
    pub fn new(config: &AlpacaConfig) -> Result<Self, ExecutionError> {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .map_err(|e| ExecutionError::Transport(e.to_string()))?;
        Ok(Self {
            base_url: config.base_url.trim_end_matches('/').to_owned(),
            api_key: config.api_key.clone(),
            api_secret: config.api_secret.clone(),
            client,
        })
    }
}

impl HttpTransport for RemoteAlpacaHttp {
    fn request(&mut self, req: &HttpRequest) -> Result<HttpResponse, ExecutionError> {
        let url = format!("{}{}", self.base_url, req.path);
        let builder = match req.method {
            HttpMethod::Get => self.client.get(&url),
            HttpMethod::Post => self.client.post(&url).body(req.body.clone()),
            HttpMethod::Patch => self.client.patch(&url).body(req.body.clone()),
            HttpMethod::Delete => self.client.delete(&url),
        };
        let resp = builder
            .header("APCA-API-KEY-ID", &self.api_key)
            .header("APCA-API-SECRET-KEY", &self.api_secret)
            .header("content-type", "application/json")
            .send()
            .map_err(|e| ExecutionError::Transport(e.to_string()))?;
        let status = resp.status().as_u16();
        let body = resp
            .text()
            .map_err(|e| ExecutionError::Transport(e.to_string()))?;
        Ok(HttpResponse { status, body })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn symbols() -> HashMap<InstrumentId, String> {
        let mut m = HashMap::new();
        m.insert(InstrumentId::from_u64(1), "AAPL".into());
        m
    }

    fn order(id: u64) -> NewVenueOrder {
        NewVenueOrder {
            order_id: OrderId::from_u64(id),
            instrument_id: InstrumentId::from_u64(1),
            side: Side::Buy,
            qty: QuantityLots::from_lots(2),
            price: PriceTicks::from_scaled(10_000),
            tif: TimeInForce::Gtc,
        }
    }

    #[test]
    fn local_mock_auto_fills() {
        let mut v = AlpacaPaperVenue::local_mock(symbols());
        v.submit(&order(1)).expect("submit");
        let reports = v.poll();
        assert_eq!(reports.len(), 2);
        assert!(matches!(reports[0].exec_type(), ExecType::New));
        assert!(matches!(reports[1].exec_type(), ExecType::Trade));
        assert_eq!(v.venue_order(OrderId::from_u64(1)).unwrap().cum_qty, 2);
    }

    #[test]
    fn local_mock_cancel() {
        let mut v = AlpacaPaperVenue::local_mock(symbols());
        // ack-only style: disable auto_fill via custom transport
        v.config.auto_fill = false;
        v.transport = AlpacaTransport::Local(LocalAlpacaHttp {
            auto_fill: false,
            fills_on_get: false,
            next_id: 1,
            orders: HashMap::new(),
        });
        v.submit(&order(3)).expect("submit");
        let _ = v.poll();
        v.cancel(OrderId::from_u64(3)).expect("cancel");
        let reports = v.poll();
        assert!(matches!(reports[0].exec_type(), ExecType::Canceled));
    }

    #[test]
    fn local_mock_async_fill_on_poll() {
        let mut v = AlpacaPaperVenue::local_mock(symbols());
        v.config.auto_fill = false;
        v.transport = AlpacaTransport::Local(LocalAlpacaHttp::async_fill());
        v.submit(&order(5)).expect("submit");
        let first = v.poll();
        assert_eq!(first.len(), 1);
        assert!(matches!(first[0].exec_type(), ExecType::New));
        // Second poll GETs the order and emits the async Trade.
        let second = v.poll();
        assert_eq!(second.len(), 1);
        assert!(matches!(second[0].exec_type(), ExecType::Trade));
        assert_eq!(v.venue_order(OrderId::from_u64(5)).unwrap().cum_qty, 2);
    }

    #[test]
    fn local_mock_broker_statement_after_fill() {
        let mut v = AlpacaPaperVenue::local_mock(symbols());
        v.submit(&order(9)).expect("submit");
        let _ = v.poll();
        let stmt = v.fetch_broker_statement().expect("statement");
        assert!(stmt.account.cash_minor < 10_000_000);
        assert!(
            stmt.positions
                .iter()
                .any(|p| p.symbol == "AAPL" && p.qty == 2),
            "expected AAPL position: {:?}",
            stmt.positions
        );
        assert!(!stmt.fills.is_empty());
    }

    #[test]
    fn ticks_to_limit_price_format() {
        assert_eq!(
            AlpacaPaperVenue::ticks_to_limit_price(PriceTicks::from_scaled(10_000)),
            "100.00"
        );
        assert_eq!(
            AlpacaPaperVenue::parse_price_to_ticks("100.00")
                .unwrap()
                .scaled(),
            10_000
        );
    }
}
