-- Alternate input markers for a commodity (e.g. "A$", "AU$").
CREATE TABLE IF NOT EXISTS commodity_aliases (
    commodity_id TEXT    NOT NULL REFERENCES commodities(id),
    alias        TEXT    NOT NULL,
    position     INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (commodity_id, alias)
);
CREATE INDEX IF NOT EXISTS idx_commodity_aliases_commodity ON commodity_aliases (commodity_id);
