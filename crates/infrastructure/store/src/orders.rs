//! Order snapshot persistence.

use sqlx::{PgPool, Postgres, Transaction};

use shinrai_instruments::{InstrumentId, PriceTicks, QuantityLots};
use shinrai_ledger::AccountId;
use shinrai_orders::{
    ClientOrderId, ExecId, Order, OrderId, OrderStatus, OrderType, Side, TimeInForce, VenueOrderId,
};

use crate::error::StoreError;

/// Side as stored in Postgres.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoredSide {
    /// Buy.
    Buy,
    /// Sell.
    Sell,
}

impl From<Side> for StoredSide {
    fn from(value: Side) -> Self {
        match value {
            Side::Buy => Self::Buy,
            Side::Sell => Self::Sell,
        }
    }
}

impl StoredSide {
    fn as_str(self) -> &'static str {
        match self {
            Self::Buy => "Buy",
            Self::Sell => "Sell",
        }
    }

    fn parse(raw: &str) -> Result<Self, StoreError> {
        match raw {
            "Buy" => Ok(Self::Buy),
            "Sell" => Ok(Self::Sell),
            other => Err(StoreError::InvalidStored {
                field: "side",
                value: other.to_owned(),
            }),
        }
    }

    /// Domain side.
    #[must_use]
    pub const fn to_side(self) -> Side {
        match self {
            Self::Buy => Side::Buy,
            Self::Sell => Side::Sell,
        }
    }
}

/// Order status as stored text (matches [`OrderStatus`] Display).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoredStatus {
    /// `PendingNew`.
    PendingNew,
    /// `New`.
    New,
    /// `PartiallyFilled`.
    PartiallyFilled,
    /// `Filled`.
    Filled,
    /// `PendingCancel`.
    PendingCancel,
    /// `Canceled`.
    Canceled,
    /// `PendingReplace`.
    PendingReplace,
    /// `Rejected`.
    Rejected,
    /// `Expired`.
    Expired,
}

impl From<OrderStatus> for StoredStatus {
    fn from(value: OrderStatus) -> Self {
        match value {
            OrderStatus::PendingNew => Self::PendingNew,
            OrderStatus::New => Self::New,
            OrderStatus::PartiallyFilled => Self::PartiallyFilled,
            OrderStatus::Filled => Self::Filled,
            OrderStatus::PendingCancel => Self::PendingCancel,
            OrderStatus::Canceled => Self::Canceled,
            OrderStatus::PendingReplace => Self::PendingReplace,
            OrderStatus::Rejected => Self::Rejected,
            OrderStatus::Expired => Self::Expired,
        }
    }
}

impl StoredStatus {
    /// Stable status label (matches [`OrderStatus`] Display).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PendingNew => "PendingNew",
            Self::New => "New",
            Self::PartiallyFilled => "PartiallyFilled",
            Self::Filled => "Filled",
            Self::PendingCancel => "PendingCancel",
            Self::Canceled => "Canceled",
            Self::PendingReplace => "PendingReplace",
            Self::Rejected => "Rejected",
            Self::Expired => "Expired",
        }
    }

    fn parse(raw: &str) -> Result<Self, StoreError> {
        match raw {
            "PendingNew" => Ok(Self::PendingNew),
            "New" => Ok(Self::New),
            "PartiallyFilled" => Ok(Self::PartiallyFilled),
            "Filled" => Ok(Self::Filled),
            "PendingCancel" => Ok(Self::PendingCancel),
            "Canceled" => Ok(Self::Canceled),
            "PendingReplace" => Ok(Self::PendingReplace),
            "Rejected" => Ok(Self::Rejected),
            "Expired" => Ok(Self::Expired),
            other => Err(StoreError::InvalidStored {
                field: "status",
                value: other.to_owned(),
            }),
        }
    }

    /// Domain status.
    #[must_use]
    pub const fn to_order_status(self) -> OrderStatus {
        match self {
            Self::PendingNew => OrderStatus::PendingNew,
            Self::New => OrderStatus::New,
            Self::PartiallyFilled => OrderStatus::PartiallyFilled,
            Self::Filled => OrderStatus::Filled,
            Self::PendingCancel => OrderStatus::PendingCancel,
            Self::Canceled => OrderStatus::Canceled,
            Self::PendingReplace => OrderStatus::PendingReplace,
            Self::Rejected => OrderStatus::Rejected,
            Self::Expired => OrderStatus::Expired,
        }
    }
}

/// Durable order row (round-trips without reconstructing private OMS fields yet).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderSnapshot {
    /// Internal id.
    pub id: OrderId,
    /// Account.
    pub account_id: AccountId,
    /// Client order id.
    pub client_order_id: ClientOrderId,
    /// Instrument.
    pub instrument_id: InstrumentId,
    /// Side.
    pub side: StoredSide,
    /// Limit or Market.
    pub order_type: OrderType,
    /// Time in force.
    pub time_in_force: TimeInForce,
    /// Status.
    pub status: StoredStatus,
    /// Order qty lots.
    pub order_qty: QuantityLots,
    /// Limit price.
    pub price: PriceTicks,
    /// Cumulative filled lots.
    pub cum_qty: QuantityLots,
    /// Leaves.
    pub leaves_qty: QuantityLots,
    /// Average fill price.
    pub avg_px: Option<PriceTicks>,
    /// Venue order id.
    pub venue_order_id: Option<VenueOrderId>,
    /// Reject reason.
    pub reject_reason: Option<String>,
    /// Seen execution ids.
    pub seen_execs: Vec<ExecId>,
}

impl OrderSnapshot {
    /// Builds a snapshot from a live OMS order.
    #[must_use]
    pub fn from_order(order: &Order) -> Self {
        Self {
            id: order.id(),
            account_id: order.account_id(),
            client_order_id: order.client_order_id().clone(),
            instrument_id: order.instrument_id(),
            side: order.side().into(),
            order_type: order.order_type(),
            time_in_force: order.time_in_force(),
            status: order.status().into(),
            order_qty: order.order_qty(),
            price: order.price(),
            cum_qty: order.cum_qty(),
            leaves_qty: order.leaves_qty(),
            avg_px: order.avg_px(),
            venue_order_id: order.venue_order_id().cloned(),
            reject_reason: order.reject_reason().map(str::to_owned),
            seen_execs: order.seen_execs().to_vec(),
        }
    }

    /// Rebuilds a domain order from this snapshot.
    ///
    /// # Errors
    ///
    /// Returns order construction / invariant errors.
    pub fn to_order(&self) -> Result<Order, shinrai_orders::OrderError> {
        Order::restore(
            self.id,
            self.account_id,
            self.client_order_id.clone(),
            self.instrument_id,
            self.side.to_side(),
            self.order_type,
            self.time_in_force,
            self.status.to_order_status(),
            self.order_qty,
            self.price,
            self.cum_qty,
            self.leaves_qty,
            self.avg_px,
            self.venue_order_id.clone(),
            self.reject_reason.clone(),
            self.seen_execs.clone(),
        )
    }
}

fn order_type_as_str(order_type: OrderType) -> &'static str {
    match order_type {
        OrderType::Limit => "Limit",
        OrderType::Market => "Market",
    }
}

fn parse_order_type(raw: &str) -> Result<OrderType, StoreError> {
    match raw {
        "Limit" => Ok(OrderType::Limit),
        "Market" => Ok(OrderType::Market),
        other => Err(StoreError::InvalidStored {
            field: "order_type",
            value: other.to_owned(),
        }),
    }
}

fn time_in_force_as_str(tif: TimeInForce) -> &'static str {
    tif.name()
}

fn parse_time_in_force(raw: &str) -> Result<TimeInForce, StoreError> {
    match raw {
        "GTC" => Ok(TimeInForce::Gtc),
        "IOC" => Ok(TimeInForce::Ioc),
        "FOK" => Ok(TimeInForce::Fok),
        other => Err(StoreError::InvalidStored {
            field: "time_in_force",
            value: other.to_owned(),
        }),
    }
}

/// Upserts an order and replaces its exec-id set.
///
/// # Errors
///
/// Returns sqlx errors.
pub async fn upsert_order(pool: &PgPool, snap: &OrderSnapshot) -> Result<(), StoreError> {
    let mut tx = pool.begin().await?;
    upsert_order_tx(&mut tx, snap).await?;
    tx.commit().await?;
    Ok(())
}

pub(crate) async fn upsert_order_tx(
    tx: &mut Transaction<'_, Postgres>,
    snap: &OrderSnapshot,
) -> Result<(), StoreError> {
    let order_type = order_type_as_str(snap.order_type);
    let time_in_force = time_in_force_as_str(snap.time_in_force);
    sqlx::query(
        r"
        INSERT INTO orders (
            id, account_id, client_order_id, instrument_id, side, order_type, time_in_force, status,
            order_qty, price_scaled, cum_qty, leaves_qty, avg_px_scaled,
            venue_order_id, reject_reason, updated_at
        ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15, NOW())
        ON CONFLICT (id) DO UPDATE SET
            account_id = EXCLUDED.account_id,
            client_order_id = EXCLUDED.client_order_id,
            instrument_id = EXCLUDED.instrument_id,
            side = EXCLUDED.side,
            order_type = EXCLUDED.order_type,
            time_in_force = EXCLUDED.time_in_force,
            status = EXCLUDED.status,
            order_qty = EXCLUDED.order_qty,
            price_scaled = EXCLUDED.price_scaled,
            cum_qty = EXCLUDED.cum_qty,
            leaves_qty = EXCLUDED.leaves_qty,
            avg_px_scaled = EXCLUDED.avg_px_scaled,
            venue_order_id = EXCLUDED.venue_order_id,
            reject_reason = EXCLUDED.reject_reason,
            updated_at = NOW()
        ",
    )
    .bind(i64::try_from(snap.id.get()).unwrap_or(i64::MAX))
    .bind(i64::try_from(snap.account_id.get()).unwrap_or(i64::MAX))
    .bind(snap.client_order_id.as_str())
    .bind(i64::try_from(snap.instrument_id.get()).unwrap_or(i64::MAX))
    .bind(snap.side.as_str())
    .bind(order_type)
    .bind(time_in_force)
    .bind(snap.status.as_str())
    .bind(snap.order_qty.lots())
    .bind(snap.price.scaled())
    .bind(snap.cum_qty.lots())
    .bind(snap.leaves_qty.lots())
    .bind(snap.avg_px.map(PriceTicks::scaled))
    .bind(snap.venue_order_id.as_ref().map(VenueOrderId::as_str))
    .bind(snap.reject_reason.as_deref())
    .execute(&mut **tx)
    .await?;

    sqlx::query("DELETE FROM order_execs WHERE order_id = $1")
        .bind(i64::try_from(snap.id.get()).unwrap_or(i64::MAX))
        .execute(&mut **tx)
        .await?;

    for exec in &snap.seen_execs {
        sqlx::query("INSERT INTO order_execs (order_id, exec_id) VALUES ($1, $2)")
            .bind(i64::try_from(snap.id.get()).unwrap_or(i64::MAX))
            .bind(exec.as_str())
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}

/// Loads an order by internal id.
///
/// # Errors
///
/// Returns sqlx / decode errors. `Ok(None)` when missing.
pub async fn load_order_by_id(
    pool: &PgPool,
    id: OrderId,
) -> Result<Option<OrderSnapshot>, StoreError> {
    let row = sqlx::query_as::<_, OrderRow>(
        r"
        SELECT id, account_id, client_order_id, instrument_id, side, order_type, time_in_force, status,
               order_qty, price_scaled, cum_qty, leaves_qty, avg_px_scaled,
               venue_order_id, reject_reason
        FROM orders WHERE id = $1
        ",
    )
    .bind(i64::try_from(id.get()).unwrap_or(i64::MAX))
    .fetch_optional(pool)
    .await?;

    match row {
        None => Ok(None),
        Some(r) => Ok(Some(row_to_snapshot(pool, r).await?)),
    }
}

/// Loads an order by account + client order id (idempotency key).
///
/// # Errors
///
/// Returns sqlx / decode errors. `Ok(None)` when missing.
pub async fn load_order_by_client(
    pool: &PgPool,
    account_id: AccountId,
    client_order_id: &ClientOrderId,
) -> Result<Option<OrderSnapshot>, StoreError> {
    let row = sqlx::query_as::<_, OrderRow>(
        r"
        SELECT id, account_id, client_order_id, instrument_id, side, order_type, time_in_force, status,
               order_qty, price_scaled, cum_qty, leaves_qty, avg_px_scaled,
               venue_order_id, reject_reason
        FROM orders WHERE account_id = $1 AND client_order_id = $2
        ",
    )
    .bind(i64::try_from(account_id.get()).unwrap_or(i64::MAX))
    .bind(client_order_id.as_str())
    .fetch_optional(pool)
    .await?;

    match row {
        None => Ok(None),
        Some(r) => Ok(Some(row_to_snapshot(pool, r).await?)),
    }
}

/// Lists all orders ordered by id ascending.
///
/// # Errors
///
/// Returns sqlx / decode errors.
pub async fn list_orders(pool: &PgPool) -> Result<Vec<OrderSnapshot>, StoreError> {
    let rows = sqlx::query_as::<_, OrderRow>(
        r"
        SELECT id, account_id, client_order_id, instrument_id, side, order_type, time_in_force, status,
               order_qty, price_scaled, cum_qty, leaves_qty, avg_px_scaled,
               venue_order_id, reject_reason
        FROM orders ORDER BY id ASC
        ",
    )
    .fetch_all(pool)
    .await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(row_to_snapshot(pool, row).await?);
    }
    Ok(out)
}

/// Returns true when durable OMS or ledger rows exist.
///
/// # Errors
///
/// Returns sqlx errors.
pub async fn has_durable_state(pool: &PgPool) -> Result<bool, StoreError> {
    let (orders,): (i64,) = sqlx::query_as("SELECT COUNT(*)::bigint FROM orders")
        .fetch_one(pool)
        .await?;
    if orders > 0 {
        return Ok(true);
    }
    let (ledger,): (i64,) = sqlx::query_as("SELECT COUNT(*)::bigint FROM ledger_entries")
        .fetch_one(pool)
        .await?;
    Ok(ledger > 0)
}

#[derive(Debug, sqlx::FromRow)]
struct OrderRow {
    id: i64,
    account_id: i64,
    client_order_id: String,
    instrument_id: i64,
    side: String,
    order_type: String,
    time_in_force: String,
    status: String,
    order_qty: i64,
    price_scaled: i64,
    cum_qty: i64,
    leaves_qty: i64,
    avg_px_scaled: Option<i64>,
    venue_order_id: Option<String>,
    reject_reason: Option<String>,
}

async fn row_to_snapshot(pool: &PgPool, row: OrderRow) -> Result<OrderSnapshot, StoreError> {
    let order_type = parse_order_type(&row.order_type)?;
    let time_in_force = parse_time_in_force(&row.time_in_force)?;
    let exec_rows: Vec<(String,)> =
        sqlx::query_as("SELECT exec_id FROM order_execs WHERE order_id = $1 ORDER BY exec_id")
            .bind(row.id)
            .fetch_all(pool)
            .await?;
    let mut seen_execs = Vec::with_capacity(exec_rows.len());
    for (exec_id,) in exec_rows {
        seen_execs.push(ExecId::new(exec_id).map_err(|_| StoreError::InvalidStored {
            field: "exec_id",
            value: String::new(),
        })?);
    }
    Ok(OrderSnapshot {
        id: OrderId::from_u64(u64::try_from(row.id).unwrap_or(0)),
        account_id: AccountId::from_u64(u64::try_from(row.account_id).unwrap_or(0)),
        client_order_id: ClientOrderId::new(row.client_order_id).map_err(|_| {
            StoreError::InvalidStored {
                field: "client_order_id",
                value: String::new(),
            }
        })?,
        instrument_id: InstrumentId::from_u64(u64::try_from(row.instrument_id).unwrap_or(0)),
        side: StoredSide::parse(&row.side)?,
        order_type,
        time_in_force,
        status: StoredStatus::parse(&row.status)?,
        order_qty: QuantityLots::from_lots(row.order_qty),
        price: PriceTicks::from_scaled(row.price_scaled),
        cum_qty: QuantityLots::from_lots(row.cum_qty),
        leaves_qty: QuantityLots::from_lots(row.leaves_qty),
        avg_px: row.avg_px_scaled.map(PriceTicks::from_scaled),
        venue_order_id: match row.venue_order_id {
            Some(v) => Some(VenueOrderId::new(v).map_err(|_| StoreError::InvalidStored {
                field: "venue_order_id",
                value: String::new(),
            })?),
            None => None,
        },
        reject_reason: row.reject_reason,
        seen_execs,
    })
}
