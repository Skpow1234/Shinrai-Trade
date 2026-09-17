-- Tamper-evident audit hash chain + correlation ids.

ALTER TABLE audit_records
    ADD COLUMN IF NOT EXISTS correlation_id TEXT;

ALTER TABLE audit_records
    ADD COLUMN IF NOT EXISTS prev_hash TEXT NOT NULL DEFAULT '';

ALTER TABLE audit_records
    ADD COLUMN IF NOT EXISTS content_hash TEXT NOT NULL DEFAULT '';
