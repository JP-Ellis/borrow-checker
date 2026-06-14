-- Remove envelope infrastructure now that budgets are account-anchored.
-- Migration 0016 already created the budgets and budget_allocations tables.

DROP TABLE IF EXISTS envelope_allocations;
DROP TABLE IF EXISTS envelope_account_links;
DROP TABLE IF EXISTS envelope_tags;
DROP TABLE IF EXISTS envelopes;

-- Remove the envelope_id column from postings.
-- Postings are now categorised solely via account_id.
-- Drop the index first (before the column it references is removed).
DROP INDEX IF EXISTS idx_postings_envelope;
ALTER TABLE postings DROP COLUMN envelope_id;
