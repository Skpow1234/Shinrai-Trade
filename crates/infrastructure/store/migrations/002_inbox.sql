-- Consumer inbox for at-least-once outbox delivery (dedup by consumer + event id).
CREATE TABLE IF NOT EXISTS inbox_dedup (
    consumer TEXT NOT NULL,
    event_id BIGINT NOT NULL REFERENCES outbox_events (id),
    processed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (consumer, event_id)
);
