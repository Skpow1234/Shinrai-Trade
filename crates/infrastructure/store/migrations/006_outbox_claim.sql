!-- Outbox multi-publisher claim stamps (SKIP LOCKED).

ALTER TABLE outbox_events
    ADD COLUMN IF NOT EXISTS claimed_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS claimed_by TEXT;

CREATE INDEX IF NOT EXISTS outbox_events_claim_idx
    ON outbox_events (id)
    WHERE published_at IS NULL;
