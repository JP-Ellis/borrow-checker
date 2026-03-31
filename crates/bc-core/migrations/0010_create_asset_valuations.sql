-- Point-in-time market valuations for ManualAsset accounts.
CREATE TABLE IF NOT EXISTS asset_valuations (
    id           TEXT PRIMARY KEY NOT NULL,
    account_id   TEXT NOT NULL REFERENCES accounts(id),
    market_value TEXT NOT NULL,  -- decimal string (positive)
    commodity    TEXT NOT NULL,  -- e.g. "AUD"
    source       TEXT NOT NULL,  -- ValuationSource snake_case string
    recorded_at  TEXT NOT NULL,  -- YYYY-MM-DD: business date of the assessment
    created_at   TEXT NOT NULL   -- ISO-8601 timestamp: when record was inserted
);

CREATE INDEX IF NOT EXISTS asset_valuations_account_recorded
    ON asset_valuations(account_id, recorded_at DESC);
