-- Append-only event log. Never update or delete rows.
CREATE TABLE IF NOT EXISTS events (
    id           TEXT NOT NULL PRIMARY KEY,
    kind         TEXT NOT NULL,
    aggregate_id TEXT NOT NULL,
    payload      TEXT NOT NULL, -- JSON
    created_at   TEXT NOT NULL  -- RFC 3339 (jiff::Timestamp serialised)
);
CREATE INDEX IF NOT EXISTS idx_events_aggregate_id ON events (aggregate_id);
CREATE INDEX IF NOT EXISTS idx_events_kind          ON events (kind);
