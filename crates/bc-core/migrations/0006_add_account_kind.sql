ALTER TABLE accounts
    ADD COLUMN kind TEXT NOT NULL DEFAULT 'deposit_account';
