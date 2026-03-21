-- Running balance cache per (account, commodity).
CREATE TABLE IF NOT EXISTS balances (
    account_id TEXT NOT NULL,
    commodity  TEXT NOT NULL,
    amount     TEXT NOT NULL, -- decimal string
    updated_at TEXT NOT NULL,
    PRIMARY KEY (account_id, commodity)
);
