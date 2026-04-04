-- Loan terms for Receivable accounts.
-- Only the latest row per account_id is authoritative (last write wins).
CREATE TABLE IF NOT EXISTS loan_terms (
    id                   TEXT PRIMARY KEY NOT NULL,
    account_id           TEXT NOT NULL REFERENCES accounts(id),
    principal            TEXT NOT NULL,     -- decimal string
    interest_rate        TEXT NOT NULL,     -- annual rate as decimal fraction e.g. "0.065"
    start_date           TEXT NOT NULL,     -- YYYY-MM-DD
    term_months          INTEGER NOT NULL,
    repayment_frequency  TEXT NOT NULL,     -- JSON-encoded bc_models::Period (e.g. "monthly", "weekly", {"fortnightly":{"anchor":"YYYY-MM-DD"}})
    commodity            TEXT NOT NULL,
    created_at           TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS loan_terms_account_id
    ON loan_terms(account_id, created_at DESC);
