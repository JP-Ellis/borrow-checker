-- Budget versioning: anchor + revision timeline. No data preserved (pre-live).

DROP TABLE IF EXISTS budget_allocations;
DROP INDEX IF EXISTS budgets_account_untagged;
DROP INDEX IF EXISTS budgets_account_tagged;
DROP INDEX IF EXISTS idx_budgets_account;
DROP TABLE IF EXISTS budgets;

CREATE TABLE budgets (
    id          TEXT NOT NULL PRIMARY KEY,
    account_id  TEXT NOT NULL REFERENCES accounts(id) ON DELETE RESTRICT,
    created_at  TEXT NOT NULL,
    archived_at TEXT
);
CREATE INDEX idx_budgets_account ON budgets(account_id);

CREATE TABLE budget_revisions (
    id              TEXT NOT NULL PRIMARY KEY,
    budget_id       TEXT NOT NULL REFERENCES budgets(id) ON DELETE CASCADE,
    effective_from  TEXT NOT NULL,           -- YYYY-MM-DD exact date
    name            TEXT,
    target_amount   TEXT,                     -- decimal string; NULL = tracking-only
    target_currency TEXT,                     -- CommodityCode; NULL iff target_amount NULL
    period          TEXT NOT NULL,            -- JSON-serialised bc_models::Period
    rollover        TEXT NOT NULL
        CHECK (rollover IN ('carry_forward', 'reset_to_zero', 'cap_at_target')),
    tag_filter      TEXT REFERENCES tags(id) ON DELETE RESTRICT,
    created_at      TEXT NOT NULL,
    CHECK ((target_amount IS NULL) = (target_currency IS NULL)),
    UNIQUE (budget_id, effective_from)
);
CREATE INDEX idx_budget_revisions_budget ON budget_revisions(budget_id, effective_from);
