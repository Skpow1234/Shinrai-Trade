//! Paper trading venue handle (sim or sandbox broker).

use shinrai_exchange_simulator::{FaultConfig, SimExchange};
use shinrai_execution::{
    ExecutionError, ExecutionReport, ExecutionVenue, NewVenueOrder, SandboxBroker, SandboxConfig,
    VenueOrderSnapshot,
};
use shinrai_orders::OrderId;

/// Which in-process venue backs the paper engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VenueKind {
    /// Deterministic exchange simulator (fault injection).
    Sim,
    /// In-process broker sandbox (ack + optional auto-fill).
    Sandbox,
}

/// Owned venue implementing [`ExecutionVenue`].
#[derive(Debug, Clone)]
pub(crate) enum VenueHandle {
    Sim(SimExchange),
    Sandbox(SandboxBroker),
}

impl VenueHandle {
    pub(crate) fn sim(faults: FaultConfig) -> Self {
        Self::Sim(SimExchange::new(faults))
    }

    pub(crate) fn sandbox(config: SandboxConfig) -> Self {
        Self::Sandbox(SandboxBroker::new(config))
    }

    pub(crate) const fn kind(&self) -> VenueKind {
        match self {
            Self::Sim(_) => VenueKind::Sim,
            Self::Sandbox(_) => VenueKind::Sandbox,
        }
    }

    pub(crate) fn as_sim(&self) -> Option<&SimExchange> {
        match self {
            Self::Sim(s) => Some(s),
            Self::Sandbox(_) => None,
        }
    }

    pub(crate) fn as_sandbox_mut(&mut self) -> Option<&mut SandboxBroker> {
        match self {
            Self::Sandbox(s) => Some(s),
            Self::Sim(_) => None,
        }
    }

    pub(crate) fn submit(&mut self, order: &NewVenueOrder) -> Result<(), ExecutionError> {
        match self {
            Self::Sim(s) => ExecutionVenue::submit(s, order),
            Self::Sandbox(s) => ExecutionVenue::submit(s, order),
        }
    }

    pub(crate) fn cancel(&mut self, order_id: OrderId) -> Result<(), ExecutionError> {
        match self {
            Self::Sim(s) => ExecutionVenue::cancel(s, order_id),
            Self::Sandbox(s) => ExecutionVenue::cancel(s, order_id),
        }
    }

    pub(crate) fn poll(&mut self) -> Vec<ExecutionReport> {
        match self {
            Self::Sim(s) => ExecutionVenue::poll(s),
            Self::Sandbox(s) => ExecutionVenue::poll(s),
        }
    }

    pub(crate) fn tick(&mut self, ticks: u64) {
        match self {
            Self::Sim(s) => ExecutionVenue::tick(s, ticks),
            Self::Sandbox(s) => ExecutionVenue::tick(s, ticks),
        }
    }

    pub(crate) fn venue_order(&self, order_id: OrderId) -> Option<VenueOrderSnapshot> {
        match self {
            Self::Sim(s) => ExecutionVenue::venue_order(s, order_id),
            Self::Sandbox(s) => ExecutionVenue::venue_order(s, order_id),
        }
    }

    pub(crate) fn venue_orders(&self) -> Vec<VenueOrderSnapshot> {
        match self {
            Self::Sim(s) => ExecutionVenue::venue_orders(s),
            Self::Sandbox(s) => ExecutionVenue::venue_orders(s),
        }
    }
}
