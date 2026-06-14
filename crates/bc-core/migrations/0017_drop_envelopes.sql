-- Remove envelope infrastructure now that budgets are account-anchored.
-- Migration 0016 already created the budgets and budget_allocations tables.

-- Remove envelope_id from postings before dropping the envelopes table.
-- SQLite enforces FK constraints when the referenced table is dropped, so the
-- FK reference in postings must be cleared first to avoid SQLITE_CONSTRAINT_FOREIGNKEY.
DROP INDEX IF EXISTS idx_postings_envelope;
ALTER TABLE postings DROP COLUMN envelope_id;

DROP TABLE IF EXISTS envelope_allocations;
DROP TABLE IF EXISTS envelope_account_links;
DROP TABLE IF EXISTS envelope_tags;
DROP TABLE IF EXISTS envelopes;
