//! Idempotent in-memory order store (`account_id` + `client_order_id`).

use std::collections::HashMap;

use shinrai_instruments::{InstrumentId, PriceTicks, QuantityLots};
use shinrai_ledger::AccountId;

use crate::error::OrderError;
use crate::event::{DomainEffect, OrderEvent};
use crate::ids::{ClientOrderId, OrderId};
use crate::order::{Order, Side};
use crate::transition::apply;

/// Request to create a new order.
#[derive(Debug, Clone)]
pub struct CreateOrder {
    /// Account submitting the order.
    pub account_id: AccountId,
    /// Client order id (idempotency key with account).
    pub client_order_id: ClientOrderId,
    /// Instrument.
    pub instrument_id: InstrumentId,
    /// Side.
    pub side: Side,
    /// Quantity in lots.
    pub order_qty: QuantityLots,
    /// Limit price in ticks.
    pub price: PriceTicks,
}

/// Outcome of [`OrderStore::submit`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubmitOutcome {
    /// Fresh order created in `PendingNew`.
    Created(Order),
    /// Existing order returned for the same account + client order id.
    Duplicate(Order),
}

/// In-memory OMS store with append-only event log.
#[derive(Debug, Default, Clone)]
pub struct OrderStore {
    next_id: u64,
    orders: HashMap<OrderId, Order>,
    by_client: HashMap<(AccountId, ClientOrderId), OrderId>,
    /// Append-only log of applied events (including creation markers).
    event_log: Vec<(OrderId, LoggedEvent)>,
}

/// Logged store events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoggedEvent {
    /// Order created (`PendingNew`).
    Created,
    /// Domain event applied.
    Applied(OrderEvent),
}

impl OrderStore {
    /// Creates an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of orders.
    #[must_use]
    pub fn len(&self) -> usize {
        self.orders.len()
    }

    /// Returns true if the store has no orders.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.orders.is_empty()
    }

    /// Looks up an order by account + client order id (idempotency key).
    #[must_use]
    pub fn get_by_client(
        &self,
        account_id: AccountId,
        client_order_id: &ClientOrderId,
    ) -> Option<&Order> {
        let id = self.by_client.get(&(account_id, client_order_id.clone()))?;
        self.orders.get(id)
    }

    /// Event log length.
    #[must_use]
    pub fn event_log_len(&self) -> usize {
        self.event_log.len()
    }

    /// Submits a new order. Duplicate `account_id` + `client_order_id` returns
    /// the existing order without creating a second one.
    ///
    /// # Errors
    ///
    /// Returns validation errors from [`Order::new_pending`].
    pub fn submit(&mut self, req: &CreateOrder) -> Result<SubmitOutcome, OrderError> {
        let key = (req.account_id, req.client_order_id.clone());
        if let Some(id) = self.by_client.get(&key) {
            let order = self
                .orders
                .get(id)
                .cloned()
                .ok_or(OrderError::UnknownOrder { id: *id })?;
            return Ok(SubmitOutcome::Duplicate(order));
        }

        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or(OrderError::InvalidQuantity)?;
        let id = OrderId::from_u64(self.next_id);
        let order = Order::new_pending(
            id,
            req.account_id,
            req.client_order_id.clone(),
            req.instrument_id,
            req.side,
            req.order_qty,
            req.price,
        )?;

        // Append-before-ack: log creation before exposing the order.
        self.event_log.push((id, LoggedEvent::Created));
        self.by_client.insert(key, id);
        self.orders.insert(id, order.clone());
        Ok(SubmitOutcome::Created(order))
    }

    /// Applies an event to an order, logging it before mutating state.
    ///
    /// # Errors
    ///
    /// Returns unknown-order or transition errors. On error the store is unchanged.
    pub fn apply_event(
        &mut self,
        id: OrderId,
        event: OrderEvent,
    ) -> Result<(Order, Vec<DomainEffect>), OrderError> {
        let current = self
            .orders
            .get(&id)
            .cloned()
            .ok_or(OrderError::UnknownOrder { id })?;

        // Validate transition first without mutating.
        let (next, effects) = apply(&current, &event)?;

        // Append-before-ack.
        self.event_log.push((id, LoggedEvent::Applied(event)));
        self.orders.insert(id, next.clone());
        Ok((next, effects))
    }

    /// Returns an order by id.
    ///
    /// # Errors
    ///
    /// Returns [`OrderError::UnknownOrder`] if missing.
    pub fn get(&self, id: OrderId) -> Result<&Order, OrderError> {
        self.orders.get(&id).ok_or(OrderError::UnknownOrder { id })
    }

    /// All orders in undefined order (for invariant checks).
    pub fn orders(&self) -> impl Iterator<Item = &Order> {
        self.orders.values()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{ExecId, VenueOrderId};
    use crate::status::OrderStatus;

    fn req(clid: &str) -> CreateOrder {
        CreateOrder {
            account_id: AccountId::from_u64(7),
            client_order_id: ClientOrderId::new(clid).expect("c"),
            instrument_id: InstrumentId::from_u64(1),
            side: Side::Buy,
            order_qty: QuantityLots::from_lots(5),
            price: PriceTicks::from_scaled(50),
        }
    }

    #[test]
    fn duplicate_client_order_id_does_not_create_second() {
        let mut store = OrderStore::new();
        let first = match store.submit(&req("abc")).expect("s1") {
            SubmitOutcome::Created(o) => o,
            SubmitOutcome::Duplicate(_) => panic!("expected created"),
        };
        let second = store.submit(&req("abc")).expect("s2");
        assert!(matches!(second, SubmitOutcome::Duplicate(_)));
        assert_eq!(store.len(), 1);
        assert_eq!(first.id(), store.get(first.id()).expect("g").id());
    }

    #[test]
    fn apply_event_logs_and_updates() {
        let mut store = OrderStore::new();
        let order = match store.submit(&req("x")).expect("s") {
            SubmitOutcome::Created(o) => o,
            SubmitOutcome::Duplicate(_) => panic!("created"),
        };
        let (order, _) = store
            .apply_event(
                order.id(),
                OrderEvent::Accepted {
                    venue_order_id: VenueOrderId::new("V").expect("v"),
                },
            )
            .expect("ack");
        assert_eq!(order.status(), OrderStatus::New);
        assert!(store.event_log_len() >= 2);

        let (order, _) = store
            .apply_event(
                order.id(),
                OrderEvent::Trade {
                    exec_id: ExecId::new("e1").expect("e"),
                    qty: QuantityLots::from_lots(5),
                    price: PriceTicks::from_scaled(50),
                },
            )
            .expect("fill");
        assert_eq!(order.status(), OrderStatus::Filled);
    }

    #[test]
    fn failed_transition_does_not_mutate() {
        let mut store = OrderStore::new();
        let order = match store.submit(&req("y")).expect("s") {
            SubmitOutcome::Created(o) => o,
            SubmitOutcome::Duplicate(_) => panic!("created"),
        };
        let log_len = store.event_log_len();
        let err = store
            .apply_event(order.id(), OrderEvent::Canceled)
            .expect_err("illegal from PendingNew");
        assert!(matches!(err, OrderError::IllegalTransition { .. }));
        assert_eq!(store.event_log_len(), log_len);
        assert_eq!(
            store.get(order.id()).expect("g").status(),
            OrderStatus::PendingNew
        );
    }
}
