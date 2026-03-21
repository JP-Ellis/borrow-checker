-- Projected read model for accounts (rebuilt from events).
CREATE TABLE IF NOT EXISTS accounts (
    id           TEXT NOT NULL PRIMARY KEY,
    name         TEXT NOT NULL,
    account_type TEXT NOT NULL,
    commodity    TEXT NOT NULL,
    description  TEXT,
    created_at   TEXT NOT NULL,
    archived_at  TEXT
);
