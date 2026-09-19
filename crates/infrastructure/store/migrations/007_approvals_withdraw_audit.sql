-- Dual-control approval requests + paper withdraw audit trail.

CREATE TABLE IF NOT EXISTS approval_requests (
    id            BIGSERIAL PRIMARY KEY,
    account_id    BIGINT NOT NULL,
    instrument_id BIGINT NOT NULL,
    symbol        TEXT NOT NULL,
    requested_by  TEXT NOT NULL,
    approved_by   TEXT,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    approved_at   TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS approval_requests_pending_idx
    ON approval_requests (id)
    WHERE approved_by IS NULL;

CREATE TABLE IF NOT EXISTS withdraw_audit (
    id               BIGSERIAL PRIMARY KEY,
    account_id       BIGINT NOT NULL,
    amount_minor     BIGINT NOT NULL CHECK (amount_minor > 0),
    currency         TEXT NOT NULL,
    idempotency_key  TEXT NOT NULL,
    actor_subject    TEXT,
    override_used    BOOLEAN NOT NULL DEFAULT FALSE,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (idempotency_key)
);

CREATE INDEX IF NOT EXISTS withdraw_audit_account_idx
    ON withdraw_audit (account_id, created_at DESC);
