//! Property tests over the paper trading loop.

use proptest::prelude::*;
use shinrai_exchange_simulator::{FaultConfig, FillPolicy};
use shinrai_instruments::{aapl, phase1_master, PriceTicks, QuantityLots};
use shinrai_ledger::AccountId;
use shinrai_money::{Currency, Money};
use shinrai_orders::{ClientOrderId, OrderId, Side, SubmitOutcome};
use shinrai_paper::{PaperEngine, SubmitRequest};

fn funded_engine(faults: FaultConfig) -> (PaperEngine, AccountId) {
    let mut engine = PaperEngine::new(phase1_master(), faults);
    let acc = AccountId::from_u64(1);
    engine
        .deposit(
            acc,
            Money::from_major(100_000, Currency::usd()).expect("deposit"),
            "dep",
        )
        .expect("deposit ok");
    (engine, acc)
}

fn assert_invariants(engine: &PaperEngine, acc: AccountId) {
    assert!(
        engine.book().journal().trial_balance_ok(),
        "trial balance must balance"
    );
    for order in engine.orders().orders() {
        order
            .assert_invariants()
            .expect("order fill accounting invariants");
        assert!(order.cum_qty().lots() <= order.order_qty().lots());
        assert!(order.leaves_qty().lots() >= 0);
    }

    let mut net_lots = 0_i64;
    for order in engine.orders().orders() {
        if order.account_id() == acc && order.instrument_id() == aapl().id() {
            let filled = order.cum_qty().lots();
            net_lots = match order.side() {
                Side::Buy => net_lots.saturating_add(filled),
                Side::Sell => net_lots.saturating_sub(filled),
            };
        }
    }
    assert_eq!(engine.book().position(acc, aapl().id()), net_lots);

    let reserved = engine.book().reserved(acc, Currency::usd());
    assert!(reserved.minor_units() >= 0);
    let available = engine.book().available(acc, Currency::usd());
    assert!(available.minor_units() >= 0);
}

#[derive(Debug, Clone)]
enum Action {
    Submit {
        client_key: u32,
        qty: i64,
        price_scaled: i64,
    },
    Cancel {
        order_id: u64,
    },
    Tick(u64),
}

fn run_actions(actions: &[Action], faults: FaultConfig) {
    let (mut engine, acc) = funded_engine(faults);
    let mut oms_clients: std::collections::HashSet<u32> = std::collections::HashSet::new();

    for action in actions {
        match action {
            Action::Submit {
                client_key,
                qty,
                price_scaled,
            } => {
                let clid = format!("c{client_key}");
                let req = SubmitRequest {
                    account_id: acc,
                    client_order_id: ClientOrderId::new(&clid).expect("clid"),
                    instrument_id: aapl().id(),
                    side: Side::Buy,
                    qty: QuantityLots::from_lots(*qty),
                    price: PriceTicks::from_scaled(*price_scaled),
                };
                let before_orders = engine.orders().len();
                let before_cash = engine.book().available(acc, Currency::usd()).minor_units();
                let before_pos = engine.book().position(acc, aapl().id());
                let before_reserved = engine.book().reserved(acc, Currency::usd()).minor_units();

                let outcome = engine.submit(&req);
                if oms_clients.contains(client_key) {
                    assert!(matches!(outcome, Ok(SubmitOutcome::Duplicate(_))));
                    assert_eq!(engine.orders().len(), before_orders);
                    assert_eq!(
                        engine.book().available(acc, Currency::usd()).minor_units(),
                        before_cash
                    );
                    assert_eq!(engine.book().position(acc, aapl().id()), before_pos);
                    assert_eq!(
                        engine.book().reserved(acc, Currency::usd()).minor_units(),
                        before_reserved
                    );
                } else if matches!(
                    outcome,
                    Ok(SubmitOutcome::Created(_) | SubmitOutcome::Duplicate(_))
                ) {
                    oms_clients.insert(*client_key);
                }
            }
            Action::Cancel { order_id } => {
                let id = OrderId::from_u64(*order_id);
                if engine.orders().get(id).is_ok() {
                    let _ = engine.cancel(id);
                }
            }
            Action::Tick(n) => {
                let _ = engine.tick(*n);
            }
        }
        assert_invariants(&engine, acc);
    }
}

prop_compose! {
    // AAPL tick is 1 scaled unit ($0.01), so any integer in range is on-grid.
    fn arb_price()(scaled in 5_000_i64..=50_000_i64) -> i64 {
        scaled
    }
}

prop_compose! {
    fn arb_action(order_count: usize)(
        kind in 0_u8..3,
        client_key in 0_u32..256,
        qty in 1_i64..=500,
        price_scaled in arb_price(),
        order_id in 1_u64..=64,
        tick_n in 1_u64..=3,
    ) -> Action {
        match kind % 3 {
            0 => Action::Submit {
                client_key,
                qty,
                price_scaled,
            },
            1 if order_count > 0 => Action::Cancel {
                order_id: order_id.min(order_count as u64),
            },
            _ => Action::Tick(tick_n),
        }
    }
}

prop_compose! {
    fn arb_action_sequence()(actions in proptest::collection::vec(arb_action(8), 1..=24)) -> Vec<Action> {
        actions
    }
}

proptest! {
    #[test]
    fn paper_loop_invariants_happy_path(actions in arb_action_sequence()) {
        run_actions(&actions, FaultConfig::happy_path());
    }

    #[test]
    fn paper_loop_invariants_rest_and_cancel(actions in arb_action_sequence()) {
        run_actions(&actions, FaultConfig {
            fill_policy: FillPolicy::Rest,
            ..FaultConfig::happy_path()
        });
    }

    #[test]
    fn paper_loop_invariants_duplicate_exec(actions in arb_action_sequence()) {
        run_actions(&actions, FaultConfig {
            duplicate_exec: true,
            ..FaultConfig::happy_path()
        });
    }

    #[test]
    fn paper_loop_invariants_split_fill(actions in arb_action_sequence()) {
        run_actions(&actions, FaultConfig {
            fill_policy: FillPolicy::Split { first_lots: 1 },
            delay_ticks: 1,
            ..FaultConfig::happy_path()
        });
    }
}
