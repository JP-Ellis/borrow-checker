-- Strengthen the budgets table with explicit FK ON DELETE behaviour,
-- CHECK constraints, and an index to speed up account-scoped queries.
--
-- SQLite cannot ALTER COLUMN or add constraints to existing columns, so the
-- table is recreated and data migrated.

PRAGMA foreign_keys = OFF;

CREATE TABLE budgets_new (
    id              TEXT NOT NULL PRIMARY KEY,
    account_id      TEXT NOT NULL REFERENCES accounts(id) ON DELETE RESTRICT,
    tag_filter      TEXT REFERENCES tags(id) ON DELETE RESTRICT,
    name            TEXT,
    target_amount   TEXT,
    target_currency TEXT,
    period          TEXT NOT NULL,
    rollover        TEXT NOT NULL
        CHECK (rollover IN ('carry_forward', 'reset_to_zero', 'cap_at_target')),
    created_at      TEXT NOT NULL,
    archived_at     TEXT,
    CHECK ((target_amount IS NULL) = (target_currency IS NULL))
);

INSERT INTO budgets_new
    SELECT id, account_id, tag_filter, name, target_amount, target_currency,
           period, rollover, created_at, archived_at
    FROM budgets;

DROP TABLE budgets;
ALTER TABLE budgets_new RENAME TO budgets;

-- Re-create the partial unique indexes dropped with the old table.
CREATE UNIQUE INDEX budgets_account_untagged
    ON budgets(account_id) WHERE tag_filter IS NULL AND archived_at IS NULL;

CREATE UNIQUE INDEX budgets_account_tagged
    ON budgets(account_id, tag_filter) WHERE tag_filter IS NOT NULL AND archived_at IS NULL;

-- Plain index for list_for_account and status queries.
CREATE INDEX idx_budgets_account ON budgets(account_id);

PRAGMA foreign_keys = ON;
