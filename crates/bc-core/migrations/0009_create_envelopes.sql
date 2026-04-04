-- Budget envelope infrastructure.
--
-- Envelopes form an arbitrary-depth tree via parent_id (self-referential).
-- There are no separate "group" entities — a parent envelope is just an
-- envelope whose children roll their actuals and allocations upward.

-- Budget envelopes (zero-based or category-tracking).
CREATE TABLE IF NOT EXISTS envelopes (
    id                          TEXT NOT NULL PRIMARY KEY,
    name                        TEXT NOT NULL,
    -- Parent envelope ID; NULL = root envelope.
    parent_id                   TEXT REFERENCES envelopes(id) ON DELETE RESTRICT,
    icon                        TEXT,
    colour                      TEXT,
    -- commodity: NULL means track across all commodities (conversion deferred to reporting).
    commodity                   TEXT,
    -- allocation_target: NULL means category tracking mode (no budget target).
    allocation_target_amount    TEXT,   -- decimal string; NULL = no target
    allocation_target_commodity TEXT,   -- CommodityCode; NULL when amount is NULL
    period                      TEXT NOT NULL,  -- JSON-serialized bc_models::Period
    rollover_policy             TEXT NOT NULL,  -- 'carry_forward' | 'reset_to_zero' | 'cap_at_target'
    created_at                  TEXT NOT NULL,
    archived_at                 TEXT
);
CREATE INDEX IF NOT EXISTS idx_envelopes_parent ON envelopes (parent_id);

-- Envelope <-> tag membership (for cross-cutting budget views).
CREATE TABLE IF NOT EXISTS envelope_tags (
    envelope_id TEXT NOT NULL REFERENCES envelopes(id) ON DELETE CASCADE,
    tag_id      TEXT NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    PRIMARY KEY (envelope_id, tag_id)
);
CREATE INDEX IF NOT EXISTS idx_envelope_tags_tag ON envelope_tags (tag_id);

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

-- Add envelope assignment to postings for budget actuals tracking.
ALTER TABLE postings ADD COLUMN envelope_id TEXT REFERENCES envelopes(id) ON DELETE SET NULL;
CREATE INDEX IF NOT EXISTS idx_postings_envelope ON postings (envelope_id);
