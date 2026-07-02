-- Display metadata for formatting amounts, migrated off the static bc-ipc
-- Currency registry. Booleans stored as INTEGER 0/1.
ALTER TABLE commodities ADD COLUMN decimals     INTEGER NOT NULL DEFAULT 2;
ALTER TABLE commodities ADD COLUMN is_iso       INTEGER NOT NULL DEFAULT 1;
ALTER TABLE commodities ADD COLUMN symbol_after INTEGER NOT NULL DEFAULT 0;
