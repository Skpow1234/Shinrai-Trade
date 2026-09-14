//! Paper trading venue handle (sim, sandbox, or REST paper broker).

use shinrai_exchange_simulator::{FaultConfig, SimExchange};
use shinrai_execution::{
    ExecutionError, ExecutionReport, ExecutionVenue, NewVenueOrder, RestPaperVenue, SandboxBroker,
    SandboxConfig, VenueOrderSnapshot, VenueSessionState, VenueTradeSnapshot,
};
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
}

/// Owned venue implementing [`ExecutionVenue`].
#[derive(Debug, Clone)]
pub(crate) enum VenueHandle {
    Sim(SimExchange),
    Sandbox(SandboxBroker),
    Rest(RestPaperVenue),
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

    pub(crate) const fn kind(&self) -> VenueKind {
        match self {
            Self::Sim(_) => VenueKind::Sim,
            Self::Sandbox(_) => VenueKind::Sandbox,
            Self::Rest(_) => VenueKind::Rest,
        }
    }

    pub(crate) fn as_sim(&self) -> Option<&SimExchange> {
        match self {
            Self::Sim(s) => Some(s),
            Self::Sandbox(_) | Self::Rest(_) => None,
        }
    }

    pub(crate) fn as_sandbox_mut(&mut self) -> Option<&mut SandboxBroker> {
        match self {
            Self::Sandbox(s) => Some(s),
            Self::Sim(_) | Self::Rest(_) => None,
        }
    }

    pub(crate) fn submit(&mut self, order: &NewVenueOrder) -> Result<(), ExecutionError> {
        match self {
            Self::Sim(s) => ExecutionVenue::submit(s, order),
            Self::Sandbox(s) => ExecutionVenue::submit(s, order),
            Self::Rest(s) => ExecutionVenue::submit(s, order),
        }
    }

    pub(crate) fn cancel(&mut self, order_id: OrderId) -> Result<(), ExecutionError> {
        match self {
            Self::Sim(s) => ExecutionVenue::cancel(s, order_id),
            Self::Sandbox(s) => ExecutionVenue::cancel(s, order_id),
            Self::Rest(s) => ExecutionVenue::cancel(s, order_id),
        }
    }

    pub(crate) fn poll(&mut self) -> Vec<ExecutionReport> {
        match self {
            Self::Sim(s) => ExecutionVenue::poll(s),
            Self::Sandbox(s) => ExecutionVenue::poll(s),
            Self::Rest(s) => ExecutionVenue::poll(s),
        }
    }

    pub(crate) fn tick(&mut self, ticks: u64) {
        match self {
            Self::Sim(s) => ExecutionVenue::tick(s, ticks),
            Self::Sandbox(s) => ExecutionVenue::tick(s, ticks),
            Self::Rest(s) => ExecutionVenue::tick(s, ticks),
        }
    }

    pub(crate) fn venue_order(&self, order_id: OrderId) -> Option<VenueOrderSnapshot> {
        match self {
            Self::Sim(s) => ExecutionVenue::venue_order(s, order_id),
            Self::Sandbox(s) => ExecutionVenue::venue_order(s, order_id),
            Self::Rest(s) => ExecutionVenue::venue_order(s, order_id),
        }
    }

    pub(crate) fn venue_orders(&self) -> Vec<VenueOrderSnapshot> {
        match self {
            Self::Sim(s) => ExecutionVenue::venue_orders(s),
            Self::Sandbox(s) => ExecutionVenue::venue_orders(s),
            Self::Rest(s) => ExecutionVenue::venue_orders(s),
        }
    }

    pub(crate) fn trade_execs(&self) -> Vec<VenueTradeSnapshot> {
        match self {
            Self::Sim(s) => ExecutionVenue::trade_execs(s),
            Self::Sandbox(s) => ExecutionVenue::trade_execs(s),
            Self::Rest(s) => ExecutionVenue::trade_execs(s),
        }
    }

    pub(crate) fn session_state(&self) -> VenueSessionState {
        match self {
            Self::Sim(s) => ExecutionVenue::session_state(s),
            Self::Sandbox(s) => ExecutionVenue::session_state(s),
            Self::Rest(s) => ExecutionVenue::session_state(s),
        }
    }

    pub(crate) fn disconnect(&mut self) {
        match self {
            Self::Sim(s) => ExecutionVenue::disconnect(s),
            Self::Sandbox(s) => ExecutionVenue::disconnect(s),
            Self::Rest(s) => ExecutionVenue::disconnect(s),
        }
    }

    pub(crate) fn reconnect(&mut self) {
        match self {
            Self::Sim(s) => ExecutionVenue::reconnect(s),
            Self::Sandbox(s) => ExecutionVenue::reconnect(s),
            Self::Rest(s) => ExecutionVenue::reconnect(s),
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
        }
    }
}
