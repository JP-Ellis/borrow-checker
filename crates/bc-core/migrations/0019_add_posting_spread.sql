-- Add accrual spread date range to postings.
-- Both columns are TEXT (YYYY-MM-DD) and nullable.
-- spread_until is inclusive (the last day of the spread).
ALTER TABLE postings ADD COLUMN spread_from  TEXT;
ALTER TABLE postings ADD COLUMN spread_until TEXT;
