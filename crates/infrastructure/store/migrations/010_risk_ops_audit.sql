-- Append-only ops audit trail for risk control-plane mutations.

CREATE TABLE IF NOT EXISTS risk_ops_audit (
    id            BIGSERIAL PRIMARY KEY,
    action        TEXT NOT NULL,
    detail_json   JSONB NOT NULL DEFAULT '{}'::jsonb,
    actor         TEXT,
    instance_id   TEXT,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS risk_ops_audit_created_idx
    ON risk_ops_audit (created_at DESC);

CREATE INDEX IF NOT EXISTS risk_ops_audit_action_idx
    ON risk_ops_audit (action, created_at DESC);
