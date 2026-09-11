-- Durable financial / OMS foundation (Phase 3.5+ / 4.3).
-- Append-only ledger and audit; orders upserted by id with client-id uniqueness.

CREATE TABLE IF NOT EXISTS orders (
    id                  BIGINT PRIMARY KEY,
    account_id          BIGINT NOT NULL,
    client_order_id     TEXT NOT NULL,
    instrument_id       BIGINT NOT NULL,
    side                TEXT NOT NULL CHECK (side IN ('Buy', 'Sell')),
    order_type          TEXT NOT NULL CHECK (order_type IN ('Limit')),
    status              TEXT NOT NULL,
    order_qty           BIGINT NOT NULL CHECK (order_qty > 0),
    price_scaled        BIGINT NOT NULL CHECK (price_scaled > 0),
    cum_qty             BIGINT NOT NULL CHECK (cum_qty >= 0),
    leaves_qty          BIGINT NOT NULL CHECK (leaves_qty >= 0),
    avg_px_scaled       BIGINT,
    venue_order_id      TEXT,
    reject_reason       TEXT,
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT orders_filled_lte_qty CHECK (cum_qty <= order_qty),
    CONSTRAINT orders_account_client_unique UNIQUE (account_id, client_order_id)
);

CREATE TABLE IF NOT EXISTS order_execs (
    order_id            BIGINT NOT NULL REFERENCES orders (id),
    exec_id             TEXT NOT NULL,
    PRIMARY KEY (order_id, exec_id)
);

CREATE TABLE IF NOT EXISTS ledger_entries (
    id                  BIGSERIAL PRIMARY KEY,
    idempotency_key     TEXT NOT NULL,
    causation_id        TEXT,
    correlation_id      TEXT,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT ledger_entries_idempotency_unique UNIQUE (idempotency_key)
);

CREATE TABLE IF NOT EXISTS ledger_postings (
    id                  BIGSERIAL PRIMARY KEY,
    entry_id            BIGINT NOT NULL REFERENCES ledger_entries (id),
    account_kind        TEXT NOT NULL,
    account_id          BIGINT,
    currency_code       CHAR(3),
    instrument_id       BIGINT,
    direction           TEXT NOT NULL CHECK (direction IN ('Debit', 'Credit')),
    -- Integer minor units as text to preserve i128 without float conversion.
    minor_units         TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS ledger_postings_entry_idx ON ledger_postings (entry_id);

CREATE TABLE IF NOT EXISTS paper_positions (
    account_id          BIGINT NOT NULL,
    instrument_id       BIGINT NOT NULL,
    lots                BIGINT NOT NULL,
    reserved_lots       BIGINT NOT NULL DEFAULT 0 CHECK (reserved_lots >= 0),
    PRIMARY KEY (account_id, instrument_id)
);

CREATE TABLE IF NOT EXISTS audit_records (
    seq                 BIGINT PRIMARY KEY,
    at_unix             BIGINT NOT NULL,
    account_id          BIGINT,
    order_id            BIGINT,
    kind                TEXT NOT NULL,
    detail              TEXT
);

CREATE TABLE IF NOT EXISTS outbox_events (
    id                  BIGSERIAL PRIMARY KEY,
    topic               TEXT NOT NULL,
    payload             JSONB NOT NULL,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    published_at        TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS outbox_unpublished_idx
    ON outbox_events (id)
    WHERE published_at IS NULL;
