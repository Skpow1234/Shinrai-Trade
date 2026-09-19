-- Durable venue Trade drop-copy + consumer session cursor.

CREATE TABLE IF NOT EXISTS drop_copy_fills (
    order_id  BIGINT NOT NULL REFERENCES orders (id),
    exec_id   TEXT NOT NULL,
    qty       BIGINT NOT NULL CHECK (qty > 0),
    price     BIGINT NOT NULL,
    session_n INT NOT NULL,
    seq       BIGINT NOT NULL CHECK (seq >= 0),
    PRIMARY KEY (order_id, exec_id)
);

CREATE INDEX IF NOT EXISTS drop_copy_fills_session_idx
    ON drop_copy_fills (session_n, seq);

CREATE TABLE IF NOT EXISTS venue_session_cursor (
    id                SMALLINT PRIMARY KEY DEFAULT 1 CHECK (id = 1),
    applied_session_n INT,
    next_expected_seq BIGINT NOT NULL DEFAULT 1 CHECK (next_expected_seq >= 1),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

INSERT INTO venue_session_cursor (id, next_expected_seq)
VALUES (1, 1)
ON CONFLICT (id) DO NOTHING;
