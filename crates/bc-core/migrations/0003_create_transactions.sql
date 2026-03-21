CREATE TABLE IF NOT EXISTS transactions (
    id          TEXT NOT NULL PRIMARY KEY,
    date        TEXT NOT NULL, -- YYYY-MM-DD
    payee       TEXT,
    description TEXT NOT NULL,
    status      TEXT NOT NULL,
    tags        TEXT NOT NULL DEFAULT '[]', -- JSON array
    created_at  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS postings (
    id             TEXT    NOT NULL PRIMARY KEY,
    transaction_id TEXT    NOT NULL REFERENCES transactions(id),
    account_id     TEXT    NOT NULL REFERENCES accounts(id),
    amount         TEXT    NOT NULL, -- decimal string
    commodity      TEXT    NOT NULL,
    memo           TEXT,
    position       INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_postings_transaction ON postings (transaction_id);
CREATE INDEX IF NOT EXISTS idx_postings_account     ON postings (account_id);
