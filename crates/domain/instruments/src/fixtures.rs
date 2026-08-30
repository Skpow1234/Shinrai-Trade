//! Phase 1 static instrument fixtures (no live vendor feed).

use shinrai_money::Currency;

use crate::error::InstrumentError;
use crate::grid::{LotSpec, TickTable};
use crate::ids::{ExternalId, InstrumentId};
use crate::instrument::Instrument;
use crate::master::InstrumentMaster;
use crate::types::{AssetClass, InstrumentStatus, InstrumentType};

/// Apple Inc. common stock — $0.01 tick, whole shares.
#[must_use]
pub fn aapl() -> Instrument {
    build_aapl().expect("AAPL fixture must be valid")
}

fn build_aapl() -> Result<Instrument, InstrumentError> {
    Instrument::new(
        InstrumentId::from_u64(1),
        "AAPL",
        AssetClass::Equity,
        InstrumentType::CommonStock,
        Currency::usd(),
        Some(Currency::usd()),
        TickTable::constant(2, 1)?, // 0.01
        LotSpec::whole_shares()?,
        1,
        InstrumentStatus::Active,
        Some("XNAS".into()),
        vec![
            ExternalId::ticker_at("AAPL", "XNAS")?,
            ExternalId::ticker("AAPL")?,
            ExternalId::isin("US0378331005")?,
        ],
    )
}

/// E-mini S&P 500 future placeholder — 0.25 tick, multiplier 50.
#[must_use]
pub fn esz5() -> Instrument {
    build_esz5().expect("ESZ5 fixture must be valid")
}

fn build_esz5() -> Result<Instrument, InstrumentError> {
    Instrument::new(
        InstrumentId::from_u64(2),
        "ESZ5",
        AssetClass::Future,
        InstrumentType::IndexFuture,
        Currency::usd(),
        Some(Currency::usd()),
        TickTable::constant(2, 25)?, // 0.25
        LotSpec::new(0, 1, 1, Some(1000), 1)?,
        50,
        InstrumentStatus::Active,
        Some("XCME".into()),
        vec![
            ExternalId::ticker_at("ESZ5", "XCME")?,
            ExternalId::ticker("ESZ5")?,
        ],
    )
}

/// BTC-USD crypto spot placeholder — $0.01 tick, 1e-8 quantity step.
#[must_use]
pub fn btc_usd() -> Instrument {
    build_btc_usd().expect("BTC-USD fixture must be valid")
}

fn build_btc_usd() -> Result<Instrument, InstrumentError> {
    Instrument::new(
        InstrumentId::from_u64(3),
        "BTC-USD",
        AssetClass::Crypto,
        InstrumentType::CryptoSpot,
        Currency::usd(),
        Some(Currency::usd()),
        TickTable::constant(2, 1)?, // 0.01 USD
        LotSpec::new(8, 1, 1, None, 1)?,
        1,
        InstrumentStatus::Active,
        None,
        vec![
            ExternalId::ticker("BTC-USD")?,
            ExternalId::new(crate::ids::IdType::BrokerSymbol, "BTCUSD", None)?,
        ],
    )
}

/// Phase 1 master containing the static fixtures.
#[must_use]
pub fn phase1_master() -> InstrumentMaster {
    let mut master = InstrumentMaster::new();
    master.insert(aapl()).expect("AAPL");
    master.insert(esz5()).expect("ESZ5");
    master.insert(btc_usd()).expect("BTC-USD");
    master
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::InstrumentStatus;

    #[test]
    fn fixtures_load_into_master() {
        let master = phase1_master();
        assert_eq!(master.len(), 3);
        assert_eq!(master.sorted_ids().len(), 3);
    }

    #[test]
    fn es_tick_and_multiplier() {
        let es = esz5();
        assert_eq!(es.multiplier(), 50);
        assert!(es.price_to_ticks("5000.00").is_ok());
        assert!(es.price_to_ticks("5000.10").is_err());
        assert!(es.price_to_ticks("5000.25").is_ok());
    }

    #[test]
    fn halted_rejects_grid() {
        let mut halted = aapl();
        // Reconstruct halted copy
        halted = Instrument::new(
            halted.id(),
            halted.symbol_display(),
            halted.asset_class(),
            halted.instrument_type(),
            halted.quote_currency(),
            halted.settle_currency(),
            halted.tick_table().clone(),
            halted.lot_spec(),
            halted.multiplier(),
            InstrumentStatus::Halted,
            halted.venue_mic().map(str::to_owned),
            halted.identifiers().to_vec(),
        )
        .expect("halted");
        let px = aapl().price_to_ticks("100.00").expect("px");
        let qty = aapl().qty_to_lots("1").expect("qty");
        assert!(matches!(
            halted.assert_order_grid(px, qty),
            Err(crate::error::InstrumentError::NotTradable { .. })
        ));
    }
}
