CREATE TABLE IF NOT EXISTS transactions (
    id          TEXT NOT NULL PRIMARY KEY,
    date        TEXT NOT NULL, -- YYYY-MM-DD
    payee       TEXT,
    description TEXT NOT NULL,
    status      TEXT NOT NULL,
    created_at  TEXT NOT NULL
    -- tags column removed; replaced by transaction_tags join table
);

CREATE TABLE IF NOT EXISTS postings (
    id                   TEXT    NOT NULL PRIMARY KEY,
    transaction_id       TEXT    NOT NULL REFERENCES transactions(id),
    account_id           TEXT    NOT NULL REFERENCES accounts(id),
    amount               TEXT    NOT NULL, -- decimal string
    commodity            TEXT    NOT NULL, -- CommodityCode (e.g. "AUD"); not FK to commodities
    memo                 TEXT,
    position             INTEGER NOT NULL DEFAULT 0,
    -- cost basis fields (all NULL if no commodity conversion)
    cost_total_value     TEXT,   -- decimal string
    cost_total_commodity TEXT,             -- CommodityCode of the cost commodity; not FK
    cost_date            TEXT,   -- YYYY-MM-DD
    cost_label           TEXT
);
CREATE INDEX IF NOT EXISTS idx_postings_transaction ON postings (transaction_id);
CREATE INDEX IF NOT EXISTS idx_postings_account     ON postings (account_id);

-- Transaction ↔ tag membership
CREATE TABLE IF NOT EXISTS transaction_tags (
    transaction_id TEXT NOT NULL REFERENCES transactions(id),
    tag_id         TEXT NOT NULL REFERENCES tags(id),
    PRIMARY KEY (transaction_id, tag_id)
);
CREATE INDEX IF NOT EXISTS idx_transaction_tags_tag ON transaction_tags (tag_id);

-- Posting ↔ tag membership
CREATE TABLE IF NOT EXISTS posting_tags (
    posting_id TEXT NOT NULL REFERENCES postings(id),
    tag_id     TEXT NOT NULL REFERENCES tags(id),
    PRIMARY KEY (posting_id, tag_id)
);
CREATE INDEX IF NOT EXISTS idx_posting_tags_tag ON posting_tags (tag_id);

-- Transaction link registry
CREATE TABLE IF NOT EXISTS transaction_links (
    id         TEXT NOT NULL PRIMARY KEY,
    link_type  TEXT NOT NULL, -- 'transfer' | 'reversal'
    created_at TEXT NOT NULL
);

-- Link ↔ transaction membership
CREATE TABLE IF NOT EXISTS transaction_link_members (
    link_id        TEXT NOT NULL REFERENCES transaction_links(id),
    transaction_id TEXT NOT NULL REFERENCES transactions(id),
    PRIMARY KEY (link_id, transaction_id)
);
