//! Explicit order state transitions.

use crate::error::OrderError;
use crate::event::{DomainEffect, OrderEvent};
use crate::order::Order;
use crate::status::OrderStatus;

/// Applies an event to an order, returning the updated order and domain effects.
///
/// # Errors
///
/// Returns [`OrderError::IllegalTransition`] or fill/replace validation errors.
pub fn apply(order: &Order, event: &OrderEvent) -> Result<(Order, Vec<DomainEffect>), OrderError> {
    let mut next = order.clone();
    let effects = match (order.status(), event) {
        (OrderStatus::PendingNew, OrderEvent::Accepted { venue_order_id }) => {
            next.set_status(OrderStatus::New);
            next.set_venue_order_id(venue_order_id.clone());
            vec![DomainEffect::Accepted {
                venue_order_id: venue_order_id.clone(),
            }]
        }
        (OrderStatus::PendingNew, OrderEvent::Rejected { reason }) => {
            next.set_status(OrderStatus::Rejected);
            next.set_reject_reason(reason.clone());
            vec![DomainEffect::Rejected {
                reason: reason.clone(),
            }]
        }

        (
            OrderStatus::New | OrderStatus::PartiallyFilled,
            OrderEvent::Trade {
                exec_id,
                qty,
                price,
            },
        ) => {
            let filled = next.apply_trade(exec_id.clone(), *qty, *price)?;
            vec![DomainEffect::Trade {
                exec_id: exec_id.clone(),
                qty: *qty,
                price: *price,
                filled,
            }]
        }

        (OrderStatus::New | OrderStatus::PartiallyFilled, OrderEvent::CancelRequested) => {
            next.set_status(OrderStatus::PendingCancel);
            vec![DomainEffect::CancelPending]
        }
        (OrderStatus::PendingCancel, OrderEvent::Canceled) => {
            next.set_status(OrderStatus::Canceled);
            next.clear_pending_replace();
            vec![DomainEffect::Canceled]
        }

        (
            OrderStatus::New | OrderStatus::PartiallyFilled,
            OrderEvent::ReplaceRequested { new_qty, new_price },
        ) => {
            if new_qty.lots() < order.cum_qty().lots() {
                return Err(OrderError::ReplaceBelowFilled {
                    new_qty: new_qty.lots(),
                    cum_qty: order.cum_qty().lots(),
                });
            }
            if new_qty.lots() <= 0 {
                return Err(OrderError::InvalidQuantity);
            }
            if new_price.scaled() <= 0 {
                return Err(OrderError::InvalidPrice);
            }
            next.set_pending_replace(*new_qty, *new_price);
            vec![DomainEffect::ReplacePending]
        }
        (OrderStatus::PendingReplace, OrderEvent::Replaced { new_qty, new_price }) => {
            next.apply_replaced(*new_qty, *new_price)?;
            vec![DomainEffect::Replaced {
                qty: *new_qty,
                price: *new_price,
            }]
        }

        (OrderStatus::New | OrderStatus::PartiallyFilled, OrderEvent::Expired) => {
            next.set_status(OrderStatus::Expired);
            vec![DomainEffect::Expired]
        }

        // Cancel can also be confirmed from working states (venue cancel without pending)
        (OrderStatus::New | OrderStatus::PartiallyFilled, OrderEvent::Canceled) => {
            next.set_status(OrderStatus::Canceled);
            vec![DomainEffect::Canceled]
        }

        _ => {
            return Err(OrderError::IllegalTransition {
                from: order.status(),
                event: event.name(),
            });
        }
    };

    next.assert_invariants()?;
    Ok((next, effects))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{ClientOrderId, ExecId, OrderId, VenueOrderId};
    use crate::order::Side;
    use shinrai_instruments::{InstrumentId, PriceTicks, QuantityLots};
    use shinrai_ledger::AccountId;

    fn seed() -> Order {
        Order::new_pending(
            OrderId::from_u64(1),
            AccountId::from_u64(1),
            ClientOrderId::new("c1").expect("id"),
            InstrumentId::from_u64(1),
            Side::Buy,
            QuantityLots::from_lots(10),
            PriceTicks::from_scaled(100),
        )
        .expect("order")
    }

    fn accept(order: &Order) -> Order {
        let (o, _) = apply(
            order,
            &OrderEvent::Accepted {
                venue_order_id: VenueOrderId::new("V1").expect("v"),
            },
        )
        .expect("accept");
        o
    }

    #[test]
    fn pending_new_to_new_to_filled() {
        let o = seed();
        let o = accept(&o);
        assert_eq!(o.status(), OrderStatus::New);
        let (o, effects) = apply(
            &o,
            &OrderEvent::Trade {
                exec_id: ExecId::new("e1").expect("e"),
                qty: QuantityLots::from_lots(10),
                price: PriceTicks::from_scaled(100),
            },
        )
        .expect("fill");
        assert_eq!(o.status(), OrderStatus::Filled);
        assert!(matches!(
            effects.as_slice(),
            [DomainEffect::Trade { filled: true, .. }]
        ));
        o.assert_invariants().expect("inv");
    }

    #[test]
    fn partial_then_fill_avg_px() {
        let o = accept(&seed());
        let (o, _) = apply(
            &o,
            &OrderEvent::Trade {
                exec_id: ExecId::new("e1").expect("e"),
                qty: QuantityLots::from_lots(4),
                price: PriceTicks::from_scaled(100),
            },
        )
        .expect("p1");
        assert_eq!(o.status(), OrderStatus::PartiallyFilled);
        let (o, _) = apply(
            &o,
            &OrderEvent::Trade {
                exec_id: ExecId::new("e2").expect("e"),
                qty: QuantityLots::from_lots(6),
                price: PriceTicks::from_scaled(110),
            },
        )
        .expect("p2");
        assert_eq!(o.status(), OrderStatus::Filled);
        // (100*4 + 110*6) / 10 = 106
        assert_eq!(o.avg_px().expect("avg").scaled(), 106);
    }

    #[test]
    fn reject_trade_after_cancel() {
        let o = accept(&seed());
        let (o, _) = apply(&o, &OrderEvent::Canceled).expect("cxl");
        let err = apply(
            &o,
            &OrderEvent::Trade {
                exec_id: ExecId::new("e1").expect("e"),
                qty: QuantityLots::from_lots(1),
                price: PriceTicks::from_scaled(100),
            },
        )
        .expect_err("illegal");
        assert!(matches!(err, OrderError::IllegalTransition { .. }));
    }

    #[test]
    fn duplicate_exec_rejected() {
        let o = accept(&seed());
        let exec = ExecId::new("e1").expect("e");
        let (o, _) = apply(
            &o,
            &OrderEvent::Trade {
                exec_id: exec.clone(),
                qty: QuantityLots::from_lots(1),
                price: PriceTicks::from_scaled(100),
            },
        )
        .expect("t1");
        let err = apply(
            &o,
            &OrderEvent::Trade {
                exec_id: exec,
                qty: QuantityLots::from_lots(1),
                price: PriceTicks::from_scaled(100),
            },
        )
        .expect_err("dup");
        assert!(matches!(err, OrderError::DuplicateExec { .. }));
    }

    #[test]
    fn exhaustive_illegal_from_terminal_filled() {
        let o = accept(&seed());
        let (o, _) = apply(
            &o,
            &OrderEvent::Trade {
                exec_id: ExecId::new("e1").expect("e"),
                qty: QuantityLots::from_lots(10),
                price: PriceTicks::from_scaled(100),
            },
        )
        .expect("fill");
        assert!(o.status().is_terminal());
        let events = [
            OrderEvent::Accepted {
                venue_order_id: VenueOrderId::new("x").expect("v"),
            },
            OrderEvent::Rejected {
                reason: "no".into(),
            },
            OrderEvent::Trade {
                exec_id: ExecId::new("e2").expect("e"),
                qty: QuantityLots::from_lots(1),
                price: PriceTicks::from_scaled(100),
            },
            OrderEvent::CancelRequested,
            OrderEvent::Canceled,
            OrderEvent::ReplaceRequested {
                new_qty: QuantityLots::from_lots(10),
                new_price: PriceTicks::from_scaled(100),
            },
            OrderEvent::Replaced {
                new_qty: QuantityLots::from_lots(10),
                new_price: PriceTicks::from_scaled(100),
            },
            OrderEvent::Expired,
        ];
        for event in events {
            assert!(
                apply(&o, &event).is_err(),
                "terminal filled must reject further events"
            );
        }
    }
}
