-- Nestable display groups for envelopes.
CREATE TABLE IF NOT EXISTS envelope_groups (
    id          TEXT NOT NULL PRIMARY KEY,
    name        TEXT NOT NULL,
    parent_id   TEXT REFERENCES envelope_groups(id) ON DELETE RESTRICT,
    created_at  TEXT NOT NULL,  -- ISO 8601 timestamp
    archived_at TEXT            -- ISO 8601 timestamp; NULL = active
);

-- Budget envelopes (zero-based or category-tracking).
CREATE TABLE IF NOT EXISTS envelopes (
    id                          TEXT NOT NULL PRIMARY KEY,
    name                        TEXT NOT NULL,
    parent_id                   TEXT REFERENCES envelope_groups(id) ON DELETE RESTRICT,
    icon                        TEXT,
    colour                      TEXT,
    -- allocation_target: NULL means category tracking mode (no budget target).
    allocation_target_amount    TEXT,   -- decimal string; NULL = no target
    allocation_target_commodity TEXT,   -- CommodityCode; NULL when amount is NULL
    period                      TEXT NOT NULL,  -- JSON-serialized bc_models::Period
    rollover_policy             TEXT NOT NULL,  -- 'carry_forward' | 'reset_to_zero' | 'cap_at_target'
    created_at                  TEXT NOT NULL,
    archived_at                 TEXT
);
CREATE INDEX IF NOT EXISTS idx_envelopes_parent ON envelopes (parent_id);

-- Linked accounts for UI hints / future auto-categorisation.
CREATE TABLE IF NOT EXISTS envelope_account_links (
    envelope_id TEXT NOT NULL REFERENCES envelopes(id) ON DELETE CASCADE,
    account_id  TEXT NOT NULL REFERENCES accounts(id) ON DELETE RESTRICT,
    PRIMARY KEY (envelope_id, account_id)
);

-- Per-period fund allocations.  One record per envelope per period.
CREATE TABLE IF NOT EXISTS envelope_allocations (
    id              TEXT NOT NULL PRIMARY KEY,
    envelope_id     TEXT NOT NULL REFERENCES envelopes(id) ON DELETE CASCADE,
    period_start    TEXT NOT NULL,  -- YYYY-MM-DD; canonical start from Period::range_containing
    amount          TEXT NOT NULL,  -- decimal string
    commodity       TEXT NOT NULL,  -- CommodityCode
    created_at      TEXT NOT NULL,
    UNIQUE (envelope_id, period_start)
);
CREATE INDEX IF NOT EXISTS idx_envelope_allocations_envelope ON envelope_allocations (envelope_id);
