-- Projected read model for accounts (rebuilt from events).
CREATE TABLE IF NOT EXISTS accounts (
    id           TEXT NOT NULL PRIMARY KEY,
    name         TEXT NOT NULL,
    account_type TEXT NOT NULL,
    -- commodity column removed; replaced by account_commodities join table
    description  TEXT,
    created_at   TEXT NOT NULL,
    archived_at  TEXT
);

-- Rich commodity registry
CREATE TABLE IF NOT EXISTS commodities (
    id           TEXT NOT NULL PRIMARY KEY,
    code         TEXT NOT NULL,
    exchange     TEXT,
    name         TEXT,
    description  TEXT,
    symbol       TEXT,
    active_from  TEXT,  -- YYYY-MM-DD
    active_until TEXT   -- YYYY-MM-DD
);

-- Allowed commodities per account; position 0 = default
CREATE TABLE IF NOT EXISTS account_commodities (
    account_id   TEXT    NOT NULL REFERENCES accounts(id),
    commodity_id TEXT    NOT NULL REFERENCES commodities(id),
    position     INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (account_id, commodity_id)
);
CREATE INDEX IF NOT EXISTS idx_account_commodities_account ON account_commodities (account_id);

-- Tag hierarchy
CREATE TABLE IF NOT EXISTS tags (
    id          TEXT NOT NULL PRIMARY KEY,
    name        TEXT NOT NULL,
    parent_id   TEXT REFERENCES tags(id),
    description TEXT,
    created_at  TEXT NOT NULL
);

-- Account ↔ tag membership
CREATE TABLE IF NOT EXISTS account_tags (
    account_id TEXT NOT NULL REFERENCES accounts(id),
    tag_id     TEXT NOT NULL REFERENCES tags(id),
    PRIMARY KEY (account_id, tag_id)
);
CREATE INDEX IF NOT EXISTS idx_account_tags_tag ON account_tags (tag_id);
CREATE INDEX IF NOT EXISTS idx_tags_parent ON tags (parent_id);
