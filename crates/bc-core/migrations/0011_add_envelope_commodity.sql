-- Add commodity column to envelopes. Existing rows default to 'AUD'.
--
-- ASSUMPTION: this migration is always co-deployed with migration 0009
-- (create_envelopes), so no rows exist yet on a fresh install.
-- If applied to an existing database that already contains non-AUD envelopes
-- (created before this column existed), their commodity will be silently set
-- to 'AUD'. In that scenario a manual data-correction step is required.
ALTER TABLE envelopes ADD COLUMN commodity TEXT NOT NULL DEFAULT 'AUD';
