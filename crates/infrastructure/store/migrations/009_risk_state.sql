-- Durable OMS risk control plane (kills, limits, restricted instruments).

CREATE TABLE IF NOT EXISTS risk_state (
    id                      SMALLINT PRIMARY KEY CHECK (id = 1),
    global_kill             BOOLEAN NOT NULL DEFAULT FALSE,
    limits_json             JSONB NOT NULL,
    account_kills           BIGINT[] NOT NULL DEFAULT '{}',
    restricted_instruments  BIGINT[] NOT NULL DEFAULT '{}',
    updated_at              TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
