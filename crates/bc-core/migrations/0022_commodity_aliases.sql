-- Alternate input markers for a commodity (e.g. "A$", "AU$").
CREATE TABLE IF NOT EXISTS commodity_aliases (
    commodity_id TEXT    NOT NULL REFERENCES commodities(id),
    alias        TEXT    NOT NULL,
    position     INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (commodity_id, alias)
);
CREATE INDEX IF NOT EXISTS idx_commodity_aliases_commodity ON commodity_aliases (commodity_id);

-- Prevent duplicate currency codes while still allowing the same `code` on
-- different `exchange`s. Currencies have `exchange IS NULL`; SQLite treats NULLs
-- as distinct under a plain UNIQUE, so guard only the no-exchange (currency) rows.
CREATE UNIQUE INDEX IF NOT EXISTS idx_commodity_code_no_exchange ON commodities (code) WHERE exchange IS NULL;
