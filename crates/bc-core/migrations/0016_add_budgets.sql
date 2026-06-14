-- Account-anchored budget lines.
--
-- Each Budget row attaches allocation targets and period rules to an account.
-- Multiple budgets per account are allowed, distinguished by tag_filter.
-- Uniqueness constraints prevent ambiguous untagged or duplicate tagged budgets.

CREATE TABLE budgets (
    id              TEXT NOT NULL PRIMARY KEY,
    account_id      TEXT NOT NULL REFERENCES accounts(id),
    tag_filter      TEXT REFERENCES tags(id),
    name            TEXT,
    -- target: NULL = tracking-only (no allocation target)
    target_amount   TEXT,   -- decimal string; NULL when no target
    target_currency TEXT,   -- CommodityCode; NULL when target_amount is NULL
    period          TEXT NOT NULL,  -- JSON-serialised bc_models::Period
    rollover        TEXT NOT NULL,  -- 'carry_forward' | 'reset_to_zero' | 'cap_at_target'
    created_at      TEXT NOT NULL,
    archived_at     TEXT
);

-- At most one untagged active budget per account (avoids unresolvable ambiguity).
CREATE UNIQUE INDEX budgets_account_untagged
    ON budgets(account_id) WHERE tag_filter IS NULL AND archived_at IS NULL;

-- At most one active tagged budget per (account, tag) pair.
CREATE UNIQUE INDEX budgets_account_tagged
    ON budgets(account_id, tag_filter) WHERE tag_filter IS NOT NULL AND archived_at IS NULL;

-- Per-period fund allocations: one record per budget per period (upsert on conflict).
CREATE TABLE budget_allocations (
    id           TEXT NOT NULL PRIMARY KEY,
    budget_id    TEXT NOT NULL REFERENCES budgets(id) ON DELETE CASCADE,
    period_start TEXT NOT NULL,  -- YYYY-MM-DD canonical period start
    amount       TEXT NOT NULL,  -- decimal string
    commodity    TEXT NOT NULL,  -- CommodityCode
    created_at   TEXT NOT NULL,
    UNIQUE (budget_id, period_start)
);
CREATE INDEX idx_budget_allocations_budget ON budget_allocations(budget_id);
