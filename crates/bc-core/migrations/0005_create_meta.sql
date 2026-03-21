-- Key-value store for global settings and schema metadata.
CREATE TABLE IF NOT EXISTS meta (
    key   TEXT NOT NULL PRIMARY KEY,
    value TEXT NOT NULL -- JSON
);
