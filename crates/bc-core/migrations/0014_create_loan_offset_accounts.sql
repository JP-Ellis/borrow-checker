CREATE TABLE loan_offset_accounts (
    loan_id    TEXT    NOT NULL,
    account_id TEXT    NOT NULL,
    created_at TEXT    NOT NULL,
    PRIMARY KEY (loan_id, account_id),
    FOREIGN KEY (loan_id)    REFERENCES loan_terms(id),
    FOREIGN KEY (account_id) REFERENCES accounts(id)
);

CREATE INDEX idx_loan_offset_accounts_loan_id
    ON loan_offset_accounts (loan_id);
