//! In-process broker sandbox that speaks [`ExecutionVenue`].
//!
//! Immediate ack + optional full fill (happy-path paper). Not a licensed
//! broker — a stand-in until a real FIX/REST adapter is wired.

use std::collections::{HashMap, VecDeque};

use shinrai_instruments::{PriceTicks, QuantityLots};
use shinrai_orders::{ExecId, OrderId, VenueOrderId};

use crate::error::ExecutionError;
use crate::report::{ExecType, ExecutionReport, SessionId};
use crate::venue::{ExecutionVenue, NewVenueOrder, VenueOrderSnapshot};

/// Sandbox behaviour knobs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SandboxConfig {
    /// When true, submit queues New then a full Trade immediately.
    pub auto_fill: bool,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self { auto_fill: true }
    }
}

impl SandboxConfig {
    /// Happy-path: ack + fill.
    #[must_use]
    pub const fn happy_path() -> Self {
        Self { auto_fill: true }
    }

    /// Ack only; fills must be injected via [`SandboxBroker::inject`].
    #[must_use]
    pub const fn ack_only() -> Self {
        Self { auto_fill: false }
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

/// Deterministic in-process sandbox venue.
#[derive(Debug, Clone)]
pub struct SandboxBroker {
    config: SandboxConfig,
    session: SessionId,
    next_seq: u64,
    next_venue: u64,
    next_exec: u64,
    inflight: HashMap<OrderId, Inflight>,
    outbox: VecDeque<ExecutionReport>,
}

impl Default for SandboxBroker {
    fn default() -> Self {
        Self::new(SandboxConfig::happy_path())
    }
}

impl SandboxBroker {
    /// Creates a connected sandbox.
    #[must_use]
    pub fn new(config: SandboxConfig) -> Self {
        Self {
            config,
            session: SessionId::new(1),
            next_seq: 1,
            next_venue: 1,
            next_exec: 1,
            inflight: HashMap::new(),
            outbox: VecDeque::new(),
        }
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
            let id = VenueOrderId::new(format!("SBX-{}", self.next_venue))
                .map_err(|_| ExecutionError::InvalidIdentifier)?;
            self.next_venue = self.next_venue.saturating_add(1);
            id
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
        self.outbox.push_back(ExecutionReport::new(
            order_id,
            venue_order_id,
            exec_id,
            exec_type,
            qty,
            price,
            self.session,
            seq,
        ));
    }

    fn alloc_venue_id(&mut self) -> Result<VenueOrderId, ExecutionError> {
        let id = VenueOrderId::new(format!("SBX-{}", self.next_venue))
            .map_err(|_| ExecutionError::InvalidIdentifier)?;
        self.next_venue = self.next_venue.saturating_add(1);
        Ok(id)
    }

    fn alloc_exec_id(&mut self) -> Result<ExecId, ExecutionError> {
        let id = ExecId::new(format!("SBX-E{}", self.next_exec))
            .map_err(|_| ExecutionError::InvalidIdentifier)?;
        self.next_exec = self.next_exec.saturating_add(1);
        Ok(id)
    }
}

impl ExecutionVenue for SandboxBroker {
    fn submit(&mut self, order: &NewVenueOrder) -> Result<(), ExecutionError> {
        if order.qty.lots() <= 0 || order.price.scaled() <= 0 {
            return Err(ExecutionError::InvalidQuantity);
        }
        let venue_order_id = self.alloc_venue_id()?;
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
        self.push_report(
            order.order_id,
            venue_order_id.clone(),
            None,
            ExecType::New,
            order.qty,
            order.price,
        );
        if self.config.auto_fill {
            let exec_id = self.alloc_exec_id()?;
            if let Some(row) = self.inflight.get_mut(&order.order_id) {
                row.cum_qty = order.qty.lots();
            }
            self.push_report(
                order.order_id,
                venue_order_id,
                Some(exec_id),
                ExecType::Trade,
                order.qty,
                order.price,
            );
        }
        Ok(())
    }

    fn cancel(&mut self, order_id: OrderId) -> Result<(), ExecutionError> {
        let Some(row) = self.inflight.get_mut(&order_id) else {
            return Err(ExecutionError::UnknownOrder { id: order_id });
        };
        if row.canceled {
            return Err(ExecutionError::InvalidState("already canceled"));
        }
        if row.cum_qty >= row.order_qty {
            return Err(ExecutionError::InvalidState("already filled"));
        }
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use shinrai_instruments::InstrumentId;
    use shinrai_orders::Side;

    #[test]
    fn auto_fill_emits_new_then_trade() {
        let mut sbx = SandboxBroker::new(SandboxConfig::happy_path());
        sbx.submit(&NewVenueOrder {
            order_id: OrderId::from_u64(1),
            instrument_id: InstrumentId::from_u64(1),
            side: Side::Buy,
            qty: QuantityLots::from_lots(5),
            price: PriceTicks::from_scaled(10_000),
        })
        .expect("submit");
        let reports = sbx.poll();
        assert_eq!(reports.len(), 2);
        assert!(matches!(reports[0].exec_type(), ExecType::New));
        assert!(matches!(reports[1].exec_type(), ExecType::Trade));
        assert_eq!(sbx.venue_order(OrderId::from_u64(1)).unwrap().cum_qty, 5);
    }
}
