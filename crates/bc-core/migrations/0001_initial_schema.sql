-- Initial consolidated schema.
--
-- This single migration is a squash of what were originally 23 incremental
-- migrations (0001–0023). The application has never been deployed and no
-- database exists in the wild, so there is no migration history to preserve:
-- the schema can be reshaped freely here without any data-migration concerns.
-- Once the app is first deployed this file becomes immutable and further
-- changes must be additive migrations (see README.md for the numbering switch
-- to timestamps at that point).
--
-- IMPORTANT — defaults: several columns below are `NOT NULL` with NO `DEFAULT`,
-- even though the original incremental migrations gave them a default. Those
-- defaults only ever existed to backfill pre-existing rows when a column was
-- retrofitted onto a populated table (SQLite requires a default to add a
-- NOT NULL column via ALTER). They are NOT part of the intended schema: every
-- INSERT supplies these values explicitly, and a silent default would mask
-- missing data. The affected columns are `commodities.decimals`,
-- `commodities.is_iso`, `commodities.symbol_after`, `accounts.kind`, and
-- `loan_terms.compounding_frequency`. Defaults that ARE genuine schema
-- defaults (e.g. `position` ordering columns, `import_profiles.config`) are
-- retained below.

-- MARK: Events

-- Append-only event log. Never update or delete rows.
CREATE TABLE events (
    id           TEXT NOT NULL PRIMARY KEY,
    kind         TEXT NOT NULL,
    aggregate_id TEXT NOT NULL,
    payload      TEXT NOT NULL, -- JSON
    created_at   TEXT NOT NULL  -- RFC 3339 (jiff::Timestamp serialised)
);
CREATE INDEX idx_events_aggregate_id ON events (aggregate_id);
CREATE INDEX idx_events_kind         ON events (kind);

-- MARK: Accounts

-- Projected read model for accounts (rebuilt from events).
--
-- `kind` has no DEFAULT: it is always written explicitly. There is
-- deliberately no CHECK constraint on it — validation lives in the domain
-- model (bc-models), not the DB layer.
--
-- The illiquid-asset metadata columns (acquisition_*, depreciation_policy)
-- are optional and only populated for asset accounts.
CREATE TABLE accounts (
    id                  TEXT NOT NULL PRIMARY KEY,
    name                TEXT NOT NULL,
    account_type        TEXT NOT NULL,
    kind                TEXT NOT NULL,
    description         TEXT,
    parent_id           TEXT REFERENCES accounts(id),
    created_at          TEXT NOT NULL,
    archived_at         TEXT,
    acquisition_date    TEXT,   -- YYYY-MM-DD when the asset was purchased
    acquisition_cost    TEXT,   -- decimal string: original purchase price
    depreciation_policy TEXT    -- JSON DepreciationPolicy; NULL = none
);
CREATE INDEX idx_accounts_name        ON accounts (name);
CREATE INDEX idx_accounts_archived_at ON accounts (archived_at);

-- No two sibling accounts share a name, so colon-separated paths resolve to at
-- most one account. COALESCE folds NULL parents (roots) to '' so roots are
-- de-duplicated too, which a plain UNIQUE would not do (SQLite treats NULLs as
-- distinct). Archived accounts participate: an archived sibling still owns its
-- name. Mirrors idx_tags_sibling_unique.
CREATE UNIQUE INDEX idx_accounts_sibling_unique
    ON accounts (COALESCE(parent_id, ''), name);

-- MARK: Commodities

-- Rich commodity registry.
--
-- `code` is NOT unique — the same ticker (e.g. "BTC", "USDT") can appear across
-- different exchanges as distinct records. Only currencies (exchange IS NULL)
-- are de-duplicated, via idx_commodity_code_no_exchange below.
--
-- Display metadata (decimals, is_iso, symbol_after) are NOT NULL with no
-- DEFAULT: every commodity specifies them. Booleans are stored as INTEGER 0/1.
CREATE TABLE commodities (
    id           TEXT NOT NULL PRIMARY KEY,
    code         TEXT NOT NULL,
    exchange     TEXT,
    name         TEXT,
    description  TEXT,
    symbol       TEXT,
    active_from  TEXT,             -- YYYY-MM-DD
    active_until TEXT,             -- YYYY-MM-DD
    decimals     INTEGER NOT NULL, -- number of fractional digits to display
    is_iso       INTEGER NOT NULL, -- 1 = ISO 4217 currency
    symbol_after INTEGER NOT NULL  -- 1 = render symbol after the amount
);
CREATE INDEX idx_commodities_code ON commodities (code);

-- Prevent duplicate currency codes while still allowing the same `code` on
-- different `exchange`s. SQLite treats NULLs as distinct under a plain UNIQUE,
-- so guard only the no-exchange (currency) rows.
CREATE UNIQUE INDEX idx_commodity_code_no_exchange
    ON commodities (code) WHERE exchange IS NULL;

-- Alternate input markers for a commodity (e.g. "A$", "AU$").
CREATE TABLE commodity_aliases (
    commodity_id TEXT    NOT NULL REFERENCES commodities(id),
    alias        TEXT    NOT NULL,
    position     INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (commodity_id, alias)
);
CREATE INDEX idx_commodity_aliases_commodity ON commodity_aliases (commodity_id);

-- Allowed commodities per account; position 0 = default.
CREATE TABLE account_commodities (
    account_id   TEXT    NOT NULL REFERENCES accounts(id),
    commodity_id TEXT    NOT NULL REFERENCES commodities(id),
    position     INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (account_id, commodity_id)
);
CREATE INDEX idx_account_commodities_account ON account_commodities (account_id);

-- MARK: Tags

-- Tag hierarchy.
CREATE TABLE tags (
    id          TEXT NOT NULL PRIMARY KEY,
    name        TEXT NOT NULL,
    parent_id   TEXT REFERENCES tags(id),
    description TEXT,
    created_at  TEXT NOT NULL
);
CREATE INDEX idx_tags_parent ON tags (parent_id);

-- No two sibling tags share a name, so computed colon-paths are unambiguous.
-- COALESCE folds NULL parents (root tags) to '' so roots are also de-duplicated.
CREATE UNIQUE INDEX idx_tags_sibling_unique
    ON tags (COALESCE(parent_id, ''), name);

-- Account <-> tag membership.
CREATE TABLE account_tags (
    account_id TEXT NOT NULL REFERENCES accounts(id),
    tag_id     TEXT NOT NULL REFERENCES tags(id),
    PRIMARY KEY (account_id, tag_id)
);
CREATE INDEX idx_account_tags_tag ON account_tags (tag_id);

-- MARK: Transactions and postings

CREATE TABLE transactions (
    id             TEXT NOT NULL PRIMARY KEY,
    date           TEXT NOT NULL, -- YYYY-MM-DD
    payee          TEXT,
    description    TEXT NOT NULL,
    note           TEXT,
    reconciliation TEXT NOT NULL,
    created_at     TEXT NOT NULL
);
CREATE INDEX idx_transactions_date           ON transactions (date);
CREATE INDEX idx_transactions_reconciliation ON transactions (reconciliation);

CREATE TABLE postings (
    id                   TEXT    NOT NULL PRIMARY KEY,
    transaction_id       TEXT    NOT NULL REFERENCES transactions(id),
    account_id           TEXT    NOT NULL REFERENCES accounts(id),
    amount               TEXT,             -- decimal string; NULL when this leg is elided
    commodity            TEXT,             -- CommodityCode (e.g. "AUD"); NULL iff amount is NULL; not FK
    note                 TEXT,
    position             INTEGER NOT NULL DEFAULT 0,
    -- Mirror of the owning transaction's date, maintained by the triggers below.
    -- Denormalised so date predicates can drive the index scan: with the date on
    -- `transactions`, SQLite applies it post-join and a six-month window costs the
    -- same as a ten-year one.
    date                 TEXT,
    -- cost basis fields (all NULL if no commodity conversion)
    cost_total_value     TEXT,             -- decimal string
    cost_total_commodity TEXT,             -- CommodityCode of the cost commodity; not FK
    cost_date            TEXT,             -- YYYY-MM-DD
    cost_label           TEXT,
    -- accrual spread date range; spread_until is inclusive
    spread_from          TEXT,             -- YYYY-MM-DD
    spread_until         TEXT,             -- YYYY-MM-DD
    CHECK ((amount IS NULL) = (commodity IS NULL))
);
CREATE INDEX idx_postings_transaction             ON postings (transaction_id);
CREATE INDEX idx_postings_account_commodity_date   ON postings (account_id, commodity, date);
CREATE INDEX idx_postings_account_date             ON postings (account_id, date);

-- These are the first triggers in this schema. `postings_date_on_insert` updates
-- `postings` from within a `postings` trigger; that is safe because SQLite's
-- `recursive_triggers` pragma is off by default, and in any case the inner statement
-- sets only `date`, which cannot fire an `AFTER UPDATE OF transaction_id` trigger.
CREATE TRIGGER postings_date_on_insert
AFTER INSERT ON postings
BEGIN
    UPDATE postings
       SET date = (SELECT t.date FROM transactions t WHERE t.id = NEW.transaction_id)
     WHERE id = NEW.id;
END;

CREATE TRIGGER postings_date_on_reparent
AFTER UPDATE OF transaction_id ON postings
BEGIN
    UPDATE postings
       SET date = (SELECT t.date FROM transactions t WHERE t.id = NEW.transaction_id)
     WHERE id = NEW.id;
END;

CREATE TRIGGER postings_date_on_transaction_date
AFTER UPDATE OF date ON transactions
BEGIN
    UPDATE postings SET date = NEW.date WHERE transaction_id = NEW.id;
END;

-- Transaction <-> tag membership.
CREATE TABLE transaction_tags (
    transaction_id TEXT NOT NULL REFERENCES transactions(id),
    tag_id         TEXT NOT NULL REFERENCES tags(id),
    PRIMARY KEY (transaction_id, tag_id)
);
CREATE INDEX idx_transaction_tags_tag ON transaction_tags (tag_id);

-- Posting <-> tag membership.
CREATE TABLE posting_tags (
    posting_id TEXT NOT NULL REFERENCES postings(id),
    tag_id     TEXT NOT NULL REFERENCES tags(id),
    PRIMARY KEY (posting_id, tag_id)
);
CREATE INDEX idx_posting_tags_tag ON posting_tags (tag_id);

-- Free-form labeled dates for a transaction.
CREATE TABLE transaction_dates (
    transaction_id TEXT NOT NULL REFERENCES transactions(id),
    label          TEXT NOT NULL,
    date           TEXT NOT NULL, -- YYYY-MM-DD
    PRIMARY KEY (transaction_id, label)
);
CREATE INDEX idx_transaction_dates_tx ON transaction_dates (transaction_id);

-- MARK: Balances

-- Running balance cache per (account, commodity). Retained as a future
-- read-cache: BalanceEngine currently computes live from `postings` and does
-- not yet read or write this table.
CREATE TABLE balances (
    account_id TEXT NOT NULL,
    commodity  TEXT NOT NULL,
    amount     TEXT NOT NULL, -- decimal string
    updated_at TEXT NOT NULL,
    PRIMARY KEY (account_id, commodity)
);

-- MARK: Meta

-- Key-value store for global settings and schema metadata.
CREATE TABLE meta (
    key   TEXT NOT NULL PRIMARY KEY,
    value TEXT NOT NULL -- JSON
);

-- MARK: Import profiles

CREATE TABLE import_profiles (
    id         TEXT PRIMARY KEY NOT NULL,
    name       TEXT NOT NULL UNIQUE,
    importer   TEXT NOT NULL,
    config     TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL
);

-- MARK: Asset valuations

-- Point-in-time market valuations for ManualAsset accounts.
CREATE TABLE asset_valuations (
    id           TEXT PRIMARY KEY NOT NULL,
    account_id   TEXT NOT NULL REFERENCES accounts(id),
    market_value TEXT NOT NULL, -- decimal string (positive)
    commodity    TEXT NOT NULL,
    source       TEXT NOT NULL, -- ValuationSource snake_case string
    recorded_at  TEXT NOT NULL, -- YYYY-MM-DD: business date of the assessment
    created_at   TEXT NOT NULL  -- ISO-8601 timestamp: when record was inserted
);
CREATE INDEX asset_valuations_account_recorded
    ON asset_valuations (account_id, recorded_at DESC);

-- MARK: Asset depreciations

-- Projection of DepreciationCalculated events.
CREATE TABLE asset_depreciations (
    id           TEXT PRIMARY KEY NOT NULL,
    account_id   TEXT NOT NULL REFERENCES accounts(id),
    amount       TEXT NOT NULL, -- positive decimal: the depreciation amount
    commodity    TEXT NOT NULL,
    period_start TEXT NOT NULL, -- YYYY-MM-DD
    period_end   TEXT NOT NULL, -- YYYY-MM-DD (inclusive)
    created_at   TEXT NOT NULL  -- ISO-8601 timestamp
);
CREATE INDEX asset_depreciations_account_period
    ON asset_depreciations (account_id, period_end DESC);

-- MARK: Loan terms

-- Loan terms for Receivable accounts. Only the latest row per account_id is
-- authoritative (last write wins). `compounding_frequency` has no DEFAULT: it
-- is always written explicitly.
CREATE TABLE loan_terms (
    id                    TEXT PRIMARY KEY NOT NULL,
    account_id            TEXT NOT NULL REFERENCES accounts(id),
    principal             TEXT NOT NULL,    -- decimal string
    interest_rate         TEXT NOT NULL,    -- annual rate as decimal fraction e.g. "0.065"
    start_date            TEXT NOT NULL,    -- YYYY-MM-DD
    term_months           INTEGER NOT NULL,
    repayment_frequency   TEXT NOT NULL,    -- JSON-encoded bc_models::Period
    commodity             TEXT NOT NULL,
    compounding_frequency TEXT NOT NULL,
    created_at            TEXT NOT NULL
);
CREATE INDEX loan_terms_account_id ON loan_terms (account_id, created_at DESC);

-- Offset accounts linked to a loan.
CREATE TABLE loan_offset_accounts (
    loan_id    TEXT NOT NULL,
    account_id TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (loan_id, account_id),
    FOREIGN KEY (loan_id)    REFERENCES loan_terms(id),
    FOREIGN KEY (account_id) REFERENCES accounts(id)
);
CREATE INDEX idx_loan_offset_accounts_loan_id ON loan_offset_accounts (loan_id);

-- MARK: Budgets

-- Account-anchored budgets with a revision timeline. A `budgets` row is a bare
-- anchor attached to an account; all mutable settings live in dated
-- `budget_revisions` so a budget's rules can change over time.
CREATE TABLE budgets (
    id          TEXT NOT NULL PRIMARY KEY,
    account_id  TEXT NOT NULL REFERENCES accounts(id) ON DELETE RESTRICT,
    created_at  TEXT NOT NULL,
    archived_at TEXT
);
CREATE INDEX idx_budgets_account ON budgets (account_id);

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
CREATE INDEX idx_budget_revisions_budget ON budget_revisions (budget_id, effective_from);

-- MARK: Import batches

-- One row per import run.
--
-- The counts are NULL until the run completes, so an aborted run is
-- distinguishable from one that finished having done nothing. The CHECK ties
-- them to finished_at, making a half-recorded outcome unrepresentable.
--
-- The skip causes are stored side by side rather than as a total plus subsets
-- of that total, so reading them back needs no subtraction.
CREATE TABLE import_batches (
    id                       TEXT NOT NULL PRIMARY KEY,
    profile_id               TEXT REFERENCES import_profiles(id) ON DELETE SET NULL,
    importer                 TEXT NOT NULL,
    started_at               TEXT NOT NULL,
    -- NULL while the run is open, and permanently if it aborted.
    finished_at              TEXT,
    -- NULL until the run is discarded. The row survives its own discard so the
    -- listing can still show what was thrown away.
    discarded_at             TEXT,
    new_transactions         INTEGER,
    attached_postings        INTEGER,
    -- Legs skipped because their account path named no existing account.
    unresolved_account_postings INTEGER,
    -- Legs skipped because their commodity code named no registered commodity.
    unresolved_commodity_postings INTEGER,
    -- Legs skipped for any other reason; each was warned about individually.
    other_skipped_postings   INTEGER,
    CHECK (
        (finished_at IS NULL) = (new_transactions IS NULL)
        AND (finished_at IS NULL) = (attached_postings IS NULL)
        AND (finished_at IS NULL) = (unresolved_account_postings IS NULL)
        AND (finished_at IS NULL) = (unresolved_commodity_postings IS NULL)
        AND (finished_at IS NULL) = (other_skipped_postings IS NULL)
    )
);

-- MARK: Transaction sources

-- Import provenance: one row per statement leg (posting) that produced a
-- transaction. Scoped to the owning account; the UNIQUE constraint is the
-- idempotency key.
--
-- A reference outlives the posting it names. Deleting the posting clears
-- posting_id rather than deleting the row, leaving a tombstone: a NULL
-- posting_id records a leg the source document contained and the user has
-- since removed. The tombstone still occupies its
-- (account_id, fingerprint, occurrence) slot, so re-importing the same
-- document does not recreate the leg.
--
-- Discarding the batch that wrote a tombstone is the one thing that removes
-- one, freeing the slot. That run never happened, so the leg it recorded is
-- not a leg the user chose to delete, and a corrected re-import has to be able
-- to recreate it.
--
-- transaction_id and account_id do cascade: removing a transaction or an
-- account takes its provenance with it, so no row can dangle.
CREATE TABLE transaction_sources (
    id             TEXT    NOT NULL PRIMARY KEY,
    transaction_id TEXT    NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
    -- NULL marks a tombstone: the leg existed in the source, and was deleted.
    posting_id     TEXT             REFERENCES postings(id)     ON DELETE SET NULL,
    account_id     TEXT    NOT NULL REFERENCES accounts(id)     ON DELETE CASCADE,
    date           TEXT    NOT NULL, -- YYYY-MM-DD
    narration      TEXT    NOT NULL,
    amount         TEXT,             -- decimal string; NULL for an elided leg
    commodity      TEXT,             -- CommodityCode; NULL for an elided leg
    reference      TEXT,             -- institution txid/reference; NULL if absent
    occurrence     INTEGER NOT NULL, -- 0-based ordinal among identical fingerprints
    fingerprint    TEXT    NOT NULL, -- canonical dedup key
    created_at     TEXT    NOT NULL,
    -- The import run that wrote this reference. NULL for references attached
    -- outside an import (SourceService::attach is public API).
    import_batch_id TEXT REFERENCES import_batches(id) ON DELETE SET NULL,
    -- Whether that import created this posting, as opposed to adopting a
    -- posting the user had already written. Discarding a batch deletes the
    -- postings it created and detaches the ones it adopted.
    owns_posting   INTEGER NOT NULL DEFAULT 0,
    UNIQUE (account_id, fingerprint, occurrence),
    -- The amount pair is the fingerprint's amount component: half of one would
    -- render a fingerprint that no longer matches the stored key.
    CHECK ((amount IS NULL) = (commodity IS NULL)),
    -- owns_posting decides whether a discard deletes a posting or merely
    -- detaches it, and its readers disagree about anything outside 0/1: the
    -- projection decodes it as `!= 0`, the discard's edit counts filter on
    -- `= 1`. A third value would delete a posting without counting it.
    CHECK (owns_posting IN (0, 1))
);

CREATE INDEX idx_transaction_sources_account_fp
    ON transaction_sources (account_id, fingerprint);

CREATE INDEX idx_transaction_sources_tx
    ON transaction_sources (transaction_id);

CREATE INDEX idx_transaction_sources_posting
    ON transaction_sources (posting_id);

CREATE INDEX idx_transaction_sources_batch
    ON transaction_sources (import_batch_id);
