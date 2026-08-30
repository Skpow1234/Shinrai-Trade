//! Execution reports (venue → OMS).

use core::fmt;

use shinrai_instruments::{PriceTicks, QuantityLots};
use shinrai_orders::{ExecId, OrderEvent, OrderId, VenueOrderId};

use crate::error::SimError;

/// Venue session identifier (`venue + session` for exec uniqueness).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SessionId {
    /// Session number (increments on reconnect/reset).
    pub n: u32,
}

impl SessionId {
    /// Session `n`.
    #[must_use]
    pub const fn new(n: u32) -> Self {
        Self { n }
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.n)
    }
}

/// FIX-like purpose of an execution report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecType {
    /// Order acknowledged (`OrdStatus=New`).
    New,
    /// Order rejected.
    Rejected {
        /// Reason.
        reason: String,
    },
    /// Trade / fill.
    Trade,
    /// Cancel confirmed.
    Canceled,
    /// Replace confirmed.
    Replaced,
    /// Cancel request rejected (order still working).
    CancelReject {
        /// Reason.
        reason: String,
    },
    /// Order expired.
    Expired,
}

impl ExecType {
    /// Stable name for fingerprints and logs.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::New => "New",
            Self::Rejected { .. } => "Rejected",
            Self::Trade => "Trade",
            Self::Canceled => "Canceled",
            Self::Replaced => "Replaced",
            Self::CancelReject { .. } => "CancelReject",
            Self::Expired => "Expired",
        }
    }
}

/// One venue execution report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionReport {
    /// Internal OMS order id (Phase 1 sim convenience).
    order_id: OrderId,
    /// Venue order id.
    venue_order_id: VenueOrderId,
    /// Execution id (required for trades).
    exec_id: Option<ExecId>,
    /// Report purpose.
    exec_type: ExecType,
    /// Last fill or current working quantity (lots).
    qty: QuantityLots,
    /// Last fill or limit price (ticks).
    price: PriceTicks,
    /// Session that produced the report.
    session: SessionId,
    /// Per-session sequence number (monotonic while connected).
    seq: u64,
}

impl ExecutionReport {
    /// Creates a report.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        order_id: OrderId,
        venue_order_id: VenueOrderId,
        exec_id: Option<ExecId>,
        exec_type: ExecType,
        qty: QuantityLots,
        price: PriceTicks,
        session: SessionId,
        seq: u64,
    ) -> Self {
        Self {
            order_id,
            venue_order_id,
            exec_id,
            exec_type,
            qty,
            price,
            session,
            seq,
        }
    }

    /// Internal order id.
    #[must_use]
    pub const fn order_id(&self) -> OrderId {
        self.order_id
    }

    /// Venue order id.
    #[must_use]
    pub fn venue_order_id(&self) -> &VenueOrderId {
        &self.venue_order_id
    }

    /// Execution id, if any.
    #[must_use]
    pub fn exec_id(&self) -> Option<&ExecId> {
        self.exec_id.as_ref()
    }

    /// Exec type.
    #[must_use]
    pub const fn exec_type(&self) -> &ExecType {
        &self.exec_type
    }

    /// Quantity.
    #[must_use]
    pub const fn qty(&self) -> QuantityLots {
        self.qty
    }

    /// Price.
    #[must_use]
    pub const fn price(&self) -> PriceTicks {
        self.price
    }

    /// Session.
    #[must_use]
    pub const fn session(&self) -> SessionId {
        self.session
    }

    /// Sequence.
    #[must_use]
    pub const fn seq(&self) -> u64 {
        self.seq
    }

    /// Maps this report to an OMS [`OrderEvent`].
    ///
    /// Cancel rejects have no order-event (OMS stays in `PendingCancel`
    /// until a later cancel ack or other terminal). Returns `Ok(None)`.
    ///
    /// # Errors
    ///
    /// Returns [`SimError::InvalidIdentifier`] if a trade is missing `exec_id`.
    pub fn to_order_event(&self) -> Result<Option<OrderEvent>, SimError> {
        match &self.exec_type {
            ExecType::New => Ok(Some(OrderEvent::Accepted {
                venue_order_id: self.venue_order_id.clone(),
            })),
            ExecType::Rejected { reason } => Ok(Some(OrderEvent::Rejected {
                reason: reason.clone(),
            })),
            ExecType::Trade => {
                let exec_id = self.exec_id.clone().ok_or(SimError::InvalidIdentifier)?;
                Ok(Some(OrderEvent::Trade {
                    exec_id,
                    qty: self.qty,
                    price: self.price,
                }))
            }
            ExecType::Canceled => Ok(Some(OrderEvent::Canceled)),
            ExecType::Replaced => Ok(Some(OrderEvent::Replaced {
                new_qty: self.qty,
                new_price: self.price,
            })),
            ExecType::CancelReject { .. } => Ok(None),
            ExecType::Expired => Ok(Some(OrderEvent::Expired)),
        }
    }

    /// Compact deterministic fingerprint fragment.
    #[must_use]
    pub fn fingerprint(&self) -> String {
        let exec = self.exec_id.as_ref().map_or("-", ExecId::as_str);
        format!(
            "{seq}:{typ}:{oid}:{vid}:{exec}:{qty}:{px}",
            seq = self.seq,
            typ = self.exec_type.name(),
            oid = self.order_id,
            vid = self.venue_order_id,
            qty = self.qty.lots(),
            px = self.price.scaled(),
        )
    }
}

impl fmt::Display for ExecutionReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.fingerprint())
    }
}

/// Fingerprint of an ordered report stream.
#[must_use]
pub fn stream_fingerprint(reports: &[ExecutionReport]) -> String {
    reports
        .iter()
        .map(ExecutionReport::fingerprint)
        .collect::<Vec<_>>()
        .join("|")
}
