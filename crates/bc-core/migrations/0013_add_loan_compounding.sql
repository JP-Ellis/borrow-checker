-- Add compounding_frequency to loan_terms.
-- Default 'monthly' preserves behaviour for existing rows (traditional model).
ALTER TABLE loan_terms
    ADD COLUMN compounding_frequency TEXT NOT NULL DEFAULT 'monthly';
