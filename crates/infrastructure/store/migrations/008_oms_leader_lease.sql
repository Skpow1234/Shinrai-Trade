-- Active-passive OMS leader lease (single row).
-- Multiple order-gateway processes elect one writer via heartbeat renewals.

CREATE TABLE IF NOT EXISTS oms_leader_lease (
    id SMALLINT PRIMARY KEY CHECK (id = 1),
    holder TEXT NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    epoch BIGINT NOT NULL DEFAULT 0
);

INSERT INTO oms_leader_lease (id, holder, expires_at, epoch)
VALUES (1, '', TIMESTAMPTZ '-infinity', 0)
ON CONFLICT (id) DO NOTHING;
