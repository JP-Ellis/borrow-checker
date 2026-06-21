-- Enforce that no two sibling tags share a name, so computed colon-paths are
-- unambiguous. COALESCE folds NULL parents (root tags) to '' so roots are also
-- de-duplicated (a plain UNIQUE index would treat each NULL as distinct).
CREATE UNIQUE INDEX IF NOT EXISTS idx_tags_sibling_unique
    ON tags (COALESCE(parent_id, ''), name);
