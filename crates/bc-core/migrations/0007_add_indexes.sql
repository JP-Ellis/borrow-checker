-- Performance indexes identified during code review.

-- accounts
CREATE INDEX IF NOT EXISTS idx_accounts_name        ON accounts (name);
CREATE INDEX IF NOT EXISTS idx_accounts_archived_at ON accounts (archived_at);

-- transactions
CREATE INDEX IF NOT EXISTS idx_transactions_date    ON transactions (date);
CREATE INDEX IF NOT EXISTS idx_transactions_status  ON transactions (status);

-- postings: balance queries filter and group by both columns
CREATE INDEX IF NOT EXISTS idx_postings_account_commodity ON postings (account_id, commodity);

-- commodities: most lookups are by code; enforce uniqueness too
CREATE UNIQUE INDEX IF NOT EXISTS idx_commodities_code ON commodities (code);

-- ---------------------------------------------------------------------------
-- Deferred constraints (tracked here for visibility)
-- ---------------------------------------------------------------------------
--
-- 1. accounts.kind CHECK constraint
--    Migration 0006 added a `kind` column but did not add a CHECK constraint
--    (e.g. CHECK (kind IN ('deposit_account', 'credit_account', ...))). A CHECK
--    constraint would enforce referential integrity at the DB layer.  Adding it
--    now requires a table rebuild, so it is deferred to a later migration.
--
-- 2. account_commodities FK to commodities(id)
--    Migration 0002 added a FK from account_commodities.commodity_id to
--    commodities(id), but no Rust code ever inserts into `commodities`, so
--    every FK reference fails at runtime.  This constraint is intentionally
--    left as-is until a CommodityService is implemented.
--
-- 3. balances table (migration 0004)
--    The `balances` table exists as a future read-cache for the balance engine.
--    Currently BalanceEngine always computes live from `postings` and never
--    reads or writes the cache.  The table is retained for the planned
--    optimisation; it should not be removed without also updating BalanceEngine.
--
-- 4. import_profiles table: deferred to Milestone 2 (Format Compatibility).
--    See DESIGN.md §4.2 and §5.3.
