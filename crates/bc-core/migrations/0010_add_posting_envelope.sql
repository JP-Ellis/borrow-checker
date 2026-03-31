-- Add envelope assignment to postings for budget actuals tracking.
ALTER TABLE postings ADD COLUMN envelope_id TEXT REFERENCES envelopes(id) ON DELETE SET NULL;
CREATE INDEX IF NOT EXISTS idx_postings_envelope ON postings (envelope_id);
