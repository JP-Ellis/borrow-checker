-- Extend accounts with optional illiquid-asset metadata.
-- acquisition_date: ISO-8601 date (YYYY-MM-DD) when the asset was purchased.
-- acquisition_cost: Decimal string — original purchase price.
-- depreciation_policy: JSON blob encoding DepreciationPolicy (null = none).
ALTER TABLE accounts ADD COLUMN acquisition_date     TEXT;
ALTER TABLE accounts ADD COLUMN acquisition_cost     TEXT;
ALTER TABLE accounts ADD COLUMN depreciation_policy  TEXT;
