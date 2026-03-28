CREATE TABLE IF NOT EXISTS import_profiles (
    id          TEXT PRIMARY KEY NOT NULL,
    name        TEXT NOT NULL,
    importer    TEXT NOT NULL,
    account_id  TEXT NOT NULL REFERENCES accounts(id),
    config      TEXT NOT NULL DEFAULT '{}',
    created_at  TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS import_profiles_account_id
    ON import_profiles(account_id);
