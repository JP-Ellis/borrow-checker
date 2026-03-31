-- Projection of DepreciationCalculated events.
-- Each row records one depreciation calculation for a ManualAsset account.
CREATE TABLE IF NOT EXISTS asset_depreciations (
    id           TEXT PRIMARY KEY NOT NULL,
    account_id   TEXT NOT NULL REFERENCES accounts(id),
    amount       TEXT NOT NULL,       -- positive decimal: the depreciation amount
    commodity    TEXT NOT NULL,
    period_start TEXT NOT NULL,       -- YYYY-MM-DD
    period_end   TEXT NOT NULL,       -- YYYY-MM-DD (inclusive)
    created_at   TEXT NOT NULL        -- ISO-8601 timestamp
);

CREATE INDEX IF NOT EXISTS asset_depreciations_account_period
    ON asset_depreciations(account_id, period_end DESC);
