//! Core [`Instrument`] definition and order-grid helpers.

use shinrai_money::Currency;

use crate::error::InstrumentError;
use crate::grid::{LotSpec, PriceTicks, QuantityLots, TickTable};
use crate::ids::{ExternalId, InstrumentId};
use crate::isin::validate_isin;
use crate::types::{AssetClass, InstrumentStatus, InstrumentType};

/// Canonical tradable instrument definition.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::struct_field_names)]
pub struct Instrument {
    id: InstrumentId,
    symbol_display: String,
    asset_class: AssetClass,
    instrument_type: InstrumentType,
    quote_currency: Currency,
    settle_currency: Option<Currency>,
    tick_table: TickTable,
    lot_spec: LotSpec,
    /// Contract multiplier (1 for equities / spot; e.g. 50 for ES).
    multiplier: i64,
    status: InstrumentStatus,
    /// Optional ISO 10383 MIC.
    venue_mic: Option<String>,
    identifiers: Vec<ExternalId>,
}

impl Instrument {
    /// Builds an instrument after validating identifiers and economics.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid MIC, ISIN, or non-positive multiplier.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: InstrumentId,
        symbol_display: impl Into<String>,
        asset_class: AssetClass,
        instrument_type: InstrumentType,
        quote_currency: Currency,
        settle_currency: Option<Currency>,
        tick_table: TickTable,
        lot_spec: LotSpec,
        multiplier: i64,
        status: InstrumentStatus,
        venue_mic: Option<String>,
        identifiers: Vec<ExternalId>,
    ) -> Result<Self, InstrumentError> {
        let symbol_display = symbol_display.into().trim().to_owned();
        if symbol_display.is_empty() {
            return Err(InstrumentError::InvalidIdentifier);
        }
        if multiplier <= 0 {
            return Err(InstrumentError::InvalidGrid("multiplier must be positive"));
        }
        let venue_mic = match venue_mic {
            Some(mic) => Some(normalize_mic(&mic)?),
            None => None,
        };
        for id_alias in &identifiers {
            if id_alias.id_type() == crate::ids::IdType::Isin {
                validate_isin(id_alias.value())?;
            }
        }
        Ok(Self {
            id,
            symbol_display,
            asset_class,
            instrument_type,
            quote_currency,
            settle_currency,
            tick_table,
            lot_spec,
            multiplier,
            status,
            venue_mic,
            identifiers,
        })
    }

    /// Returns the internal id.
    #[must_use]
    pub const fn id(&self) -> InstrumentId {
        self.id
    }

    /// Returns the display symbol.
    #[must_use]
    pub fn symbol_display(&self) -> &str {
        &self.symbol_display
    }

    /// Returns the asset class.
    #[must_use]
    pub const fn asset_class(&self) -> AssetClass {
        self.asset_class
    }

    /// Returns the instrument type.
    #[must_use]
    pub const fn instrument_type(&self) -> InstrumentType {
        self.instrument_type
    }

    /// Returns the quote currency.
    #[must_use]
    pub const fn quote_currency(&self) -> Currency {
        self.quote_currency
    }

    /// Returns the optional settle currency.
    #[must_use]
    pub const fn settle_currency(&self) -> Option<Currency> {
        self.settle_currency
    }

    /// Returns the tick table.
    #[must_use]
    pub const fn tick_table(&self) -> &TickTable {
        &self.tick_table
    }

    /// Returns the lot specification.
    #[must_use]
    pub const fn lot_spec(&self) -> LotSpec {
        self.lot_spec
    }

    /// Returns the contract multiplier.
    #[must_use]
    pub const fn multiplier(&self) -> i64 {
        self.multiplier
    }

    /// Returns trading status.
    #[must_use]
    pub const fn status(&self) -> InstrumentStatus {
        self.status
    }

    /// Returns the venue MIC if set.
    #[must_use]
    pub fn venue_mic(&self) -> Option<&str> {
        self.venue_mic.as_deref()
    }

    /// Returns external identifiers.
    #[must_use]
    pub fn identifiers(&self) -> &[ExternalId] {
        &self.identifiers
    }

    /// Ensures the instrument accepts new orders.
    ///
    /// # Errors
    ///
    /// Returns [`InstrumentError::NotTradable`] when halted/delisted/expired.
    pub fn assert_tradable(&self) -> Result<(), InstrumentError> {
        if self.status.is_tradable() {
            Ok(())
        } else {
            Err(InstrumentError::NotTradable { id: self.id })
        }
    }

    /// Parses a decimal price into on-grid ticks.
    ///
    /// # Errors
    ///
    /// Returns grid / parse errors.
    pub fn price_to_ticks(&self, decimal: &str) -> Result<PriceTicks, InstrumentError> {
        self.tick_table.price_to_ticks(decimal)
    }

    /// Formats ticks as a major-unit decimal price string.
    ///
    /// # Errors
    ///
    /// Returns overflow errors.
    pub fn ticks_to_price(&self, ticks: PriceTicks) -> Result<String, InstrumentError> {
        self.tick_table.ticks_to_price(ticks)
    }

    /// Parses a decimal quantity into lot units.
    ///
    /// # Errors
    ///
    /// Returns grid / parse errors.
    pub fn qty_to_lots(&self, decimal: &str) -> Result<QuantityLots, InstrumentError> {
        self.lot_spec.qty_to_lots(decimal)
    }

    /// Parses a book/print size into lots (`0` is a valid empty level).
    ///
    /// # Errors
    ///
    /// Returns parse / alignment errors.
    pub fn size_to_lots(&self, decimal: &str) -> Result<QuantityLots, InstrumentError> {
        self.lot_spec.size_to_lots(decimal)
    }

    /// Formats lots as a major-unit decimal quantity string.
    ///
    /// # Errors
    ///
    /// Returns overflow errors.
    pub fn lots_to_qty(&self, lots: QuantityLots) -> Result<String, InstrumentError> {
        self.lot_spec.lots_to_qty(lots)
    }

    /// Validates that a price/quantity pair is on-grid and the instrument is tradable.
    ///
    /// # Errors
    ///
    /// Returns not-tradable or grid errors.
    pub fn assert_order_grid(
        &self,
        price_ticks: PriceTicks,
        qty_lots: QuantityLots,
    ) -> Result<(), InstrumentError> {
        self.assert_tradable()?;
        self.tick_table.validate_scaled(price_ticks.scaled())?;
        let scaled_qty = qty_lots
            .lots()
            .checked_mul(self.lot_spec.step_scaled())
            .ok_or(InstrumentError::Overflow)?;
        self.lot_spec.validate_scaled(scaled_qty)?;
        Ok(())
    }
}

fn normalize_mic(mic: &str) -> Result<String, InstrumentError> {
    let mic = mic.trim();
    if mic.len() != 4 || !mic.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(InstrumentError::InvalidMic);
    }
    Ok(mic.to_ascii_uppercase())
}
