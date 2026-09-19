//! Paper trading venue handle (sim, sandbox, REST, licensed, or Alpaca paper).

use std::collections::HashMap;

use shinrai_exchange_simulator::{FaultConfig, SimExchange};
use shinrai_execution::{
    AlpacaConfig, AlpacaPaperVenue, ExecutionError, ExecutionReport, ExecutionVenue,
    LicensedSandboxConfig, LicensedSandboxVenue, NewVenueOrder, RestPaperVenue, SandboxBroker,
    SandboxConfig, VenueOrderSnapshot, VenueSessionState, VenueTradeSnapshot,
};
use shinrai_instruments::InstrumentId;
use shinrai_orders::OrderId;

/// Which in-process venue backs the paper engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VenueKind {
    /// Deterministic exchange simulator (fault injection).
    Sim,
    /// In-process broker sandbox (ack + optional auto-fill).
    Sandbox,
    /// REST-shaped paper venue (JSON over local HTTP transport).
    Rest,
    /// Session-shaped licensed sandbox (logon / heartbeats / seq recovery).
    Licensed,
    /// Alpaca paper Trading API (`/v2/orders`).
    Alpaca,
}

/// Owned venue implementing [`ExecutionVenue`].
#[derive(Debug, Clone)]
pub(crate) enum VenueHandle {
    Sim(SimExchange),
    Sandbox(SandboxBroker),
    Rest(RestPaperVenue),
    Licensed(LicensedSandboxVenue),
    Alpaca(AlpacaPaperVenue),
}

impl VenueHandle {
    pub(crate) fn sim(faults: FaultConfig) -> Self {
        Self::Sim(SimExchange::new(faults))
    }

    pub(crate) fn sandbox(config: SandboxConfig) -> Self {
        Self::Sandbox(SandboxBroker::new(config))
    }

    pub(crate) fn rest_happy_path() -> Self {
        Self::Rest(RestPaperVenue::local_happy_path())
    }

    pub(crate) fn rest_remote(
        base_url: impl Into<String>,
        bearer: Option<String>,
    ) -> Result<Self, ExecutionError> {
        Ok(Self::Rest(RestPaperVenue::remote(base_url, bearer)?))
    }

    pub(crate) fn licensed(config: LicensedSandboxConfig) -> Self {
        let mut v = LicensedSandboxVenue::new(config);
        let _ = v.logon();
        Self::Licensed(v)
    }

    pub(crate) fn alpaca_local(symbols: HashMap<InstrumentId, String>) -> Self {
        Self::Alpaca(AlpacaPaperVenue::local_mock(symbols))
    }

    pub(crate) fn alpaca_remote(
        config: AlpacaConfig,
        symbols: HashMap<InstrumentId, String>,
    ) -> Result<Self, ExecutionError> {
        Ok(Self::Alpaca(AlpacaPaperVenue::remote(config, symbols)?))
    }

    pub(crate) const fn kind(&self) -> VenueKind {
        match self {
            Self::Sim(_) => VenueKind::Sim,
            Self::Sandbox(_) => VenueKind::Sandbox,
            Self::Rest(_) => VenueKind::Rest,
            Self::Licensed(_) => VenueKind::Licensed,
            Self::Alpaca(_) => VenueKind::Alpaca,
        }
    }

    pub(crate) fn as_sim(&self) -> Option<&SimExchange> {
        match self {
            Self::Sim(s) => Some(s),
            Self::Sandbox(_) | Self::Rest(_) | Self::Licensed(_) | Self::Alpaca(_) => None,
        }
    }

    pub(crate) fn as_sandbox_mut(&mut self) -> Option<&mut SandboxBroker> {
        match self {
            Self::Sandbox(s) => Some(s),
            Self::Sim(_) | Self::Rest(_) | Self::Licensed(_) | Self::Alpaca(_) => None,
        }
    }

    pub(crate) fn as_licensed_mut(&mut self) -> Option<&mut LicensedSandboxVenue> {
        match self {
            Self::Licensed(s) => Some(s),
            Self::Sim(_) | Self::Sandbox(_) | Self::Rest(_) | Self::Alpaca(_) => None,
        }
    }

    pub(crate) fn submit(&mut self, order: &NewVenueOrder) -> Result<(), ExecutionError> {
        match self {
            Self::Sim(s) => ExecutionVenue::submit(s, order),
            Self::Sandbox(s) => ExecutionVenue::submit(s, order),
            Self::Rest(s) => ExecutionVenue::submit(s, order),
            Self::Licensed(s) => ExecutionVenue::submit(s, order),
            Self::Alpaca(s) => ExecutionVenue::submit(s, order),
        }
    }

    pub(crate) fn cancel(&mut self, order_id: OrderId) -> Result<(), ExecutionError> {
        match self {
            Self::Sim(s) => ExecutionVenue::cancel(s, order_id),
            Self::Sandbox(s) => ExecutionVenue::cancel(s, order_id),
            Self::Rest(s) => ExecutionVenue::cancel(s, order_id),
            Self::Licensed(s) => ExecutionVenue::cancel(s, order_id),
            Self::Alpaca(s) => ExecutionVenue::cancel(s, order_id),
        }
    }

    pub(crate) fn replace(
        &mut self,
        order_id: OrderId,
        new_qty: shinrai_instruments::QuantityLots,
        new_price: shinrai_instruments::PriceTicks,
    ) -> Result<(), ExecutionError> {
        match self {
            Self::Sim(s) => ExecutionVenue::replace(s, order_id, new_qty, new_price),
            Self::Sandbox(s) => ExecutionVenue::replace(s, order_id, new_qty, new_price),
            Self::Rest(s) => ExecutionVenue::replace(s, order_id, new_qty, new_price),
            Self::Licensed(s) => ExecutionVenue::replace(s, order_id, new_qty, new_price),
            Self::Alpaca(s) => ExecutionVenue::replace(s, order_id, new_qty, new_price),
        }
    }

    pub(crate) fn poll(&mut self) -> Vec<ExecutionReport> {
        match self {
            Self::Sim(s) => ExecutionVenue::poll(s),
            Self::Sandbox(s) => ExecutionVenue::poll(s),
            Self::Rest(s) => ExecutionVenue::poll(s),
            Self::Licensed(s) => ExecutionVenue::poll(s),
            Self::Alpaca(s) => ExecutionVenue::poll(s),
        }
    }

    pub(crate) fn tick(&mut self, ticks: u64) {
        match self {
            Self::Sim(s) => ExecutionVenue::tick(s, ticks),
            Self::Sandbox(s) => ExecutionVenue::tick(s, ticks),
            Self::Rest(s) => ExecutionVenue::tick(s, ticks),
            Self::Licensed(s) => ExecutionVenue::tick(s, ticks),
            Self::Alpaca(s) => ExecutionVenue::tick(s, ticks),
        }
    }

    pub(crate) fn venue_order(&self, order_id: OrderId) -> Option<VenueOrderSnapshot> {
        match self {
            Self::Sim(s) => ExecutionVenue::venue_order(s, order_id),
            Self::Sandbox(s) => ExecutionVenue::venue_order(s, order_id),
            Self::Rest(s) => ExecutionVenue::venue_order(s, order_id),
            Self::Licensed(s) => ExecutionVenue::venue_order(s, order_id),
            Self::Alpaca(s) => ExecutionVenue::venue_order(s, order_id),
        }
    }

    pub(crate) fn venue_orders(&self) -> Vec<VenueOrderSnapshot> {
        match self {
            Self::Sim(s) => ExecutionVenue::venue_orders(s),
            Self::Sandbox(s) => ExecutionVenue::venue_orders(s),
            Self::Rest(s) => ExecutionVenue::venue_orders(s),
            Self::Licensed(s) => ExecutionVenue::venue_orders(s),
            Self::Alpaca(s) => ExecutionVenue::venue_orders(s),
        }
    }

    pub(crate) fn trade_execs(&self) -> Vec<VenueTradeSnapshot> {
        match self {
            Self::Sim(s) => ExecutionVenue::trade_execs(s),
            Self::Sandbox(s) => ExecutionVenue::trade_execs(s),
            Self::Rest(s) => ExecutionVenue::trade_execs(s),
            Self::Licensed(s) => ExecutionVenue::trade_execs(s),
            Self::Alpaca(s) => ExecutionVenue::trade_execs(s),
        }
    }

    pub(crate) fn session_state(&self) -> VenueSessionState {
        match self {
            Self::Sim(s) => ExecutionVenue::session_state(s),
            Self::Sandbox(s) => ExecutionVenue::session_state(s),
            Self::Rest(s) => ExecutionVenue::session_state(s),
            Self::Licensed(s) => ExecutionVenue::session_state(s),
            Self::Alpaca(s) => ExecutionVenue::session_state(s),
        }
    }

    pub(crate) fn disconnect(&mut self) {
        match self {
            Self::Sim(s) => ExecutionVenue::disconnect(s),
            Self::Sandbox(s) => ExecutionVenue::disconnect(s),
            Self::Rest(s) => ExecutionVenue::disconnect(s),
            Self::Licensed(s) => ExecutionVenue::disconnect(s),
            Self::Alpaca(s) => ExecutionVenue::disconnect(s),
        }
    }

    pub(crate) fn reconnect(&mut self) {
        match self {
            Self::Sim(s) => ExecutionVenue::reconnect(s),
            Self::Sandbox(s) => ExecutionVenue::reconnect(s),
            Self::Rest(s) => ExecutionVenue::reconnect(s),
            Self::Licensed(s) => ExecutionVenue::reconnect(s),
            Self::Alpaca(s) => ExecutionVenue::reconnect(s),
        }
    }

    pub(crate) fn poll_recovery(
        &mut self,
        from_seq: u64,
    ) -> Result<Vec<ExecutionReport>, ExecutionError> {
        match self {
            Self::Sim(s) => ExecutionVenue::poll_recovery(s, from_seq),
            Self::Sandbox(s) => ExecutionVenue::poll_recovery(s, from_seq),
            Self::Rest(s) => ExecutionVenue::poll_recovery(s, from_seq),
            Self::Licensed(s) => ExecutionVenue::poll_recovery(s, from_seq),
            Self::Alpaca(s) => ExecutionVenue::poll_recovery(s, from_seq),
        }
    }

    /// Restores a non-terminal OMS order into the venue without new reports.
    pub(crate) fn restore_working(
        &mut self,
        order: &shinrai_orders::Order,
    ) -> Result<(), ExecutionError> {
        let cum = order.cum_qty().lots();
        let venue_id = order.venue_order_id().cloned();
        match self {
            Self::Sim(s) => s
                .restore_working(
                    order.id(),
                    order.instrument_id(),
                    order.order_qty(),
                    order.price(),
                    cum,
                    venue_id,
                )
                .map_err(Into::into),
            Self::Sandbox(s) => {
                s.restore_working(order.id(), order.order_qty(), order.price(), cum, venue_id)
            }
            Self::Rest(s) => {
                s.restore_working(order.id(), order.order_qty(), order.price(), cum, venue_id)
            }
            Self::Licensed(s) => {
                s.restore_working(order.id(), order.order_qty(), order.price(), cum, venue_id)
            }
            Self::Alpaca(s) => {
                s.restore_working(order.id(), order.order_qty(), order.price(), cum, venue_id)
            }
        }
    }
}
