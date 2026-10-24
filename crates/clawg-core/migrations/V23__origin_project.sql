-- V23: the project a memory had in the tool it came from.
--
-- NULL for every row that was not imported from another tool, which is every
-- row of a store from before this migration: migrating changes no row. The
-- name is the source's own label, never a clawg project id.
--
-- Undoing it is
--   DROP INDEX idx_memory_origin_project; DROP INDEX idx_memory_import_origin;
--   ALTER TABLE memory DROP COLUMN origin_project;
-- since SQLite refuses to drop an indexed column.
ALTER TABLE memory ADD COLUMN origin_project TEXT;

-- Choosing the rows of one project of the source.
CREATE INDEX IF NOT EXISTS idx_memory_origin_project ON memory (origin_project)
    WHERE origin_project IS NOT NULL;

-- Recognizing an imported row by the reference it kept from its source.
CREATE INDEX IF NOT EXISTS idx_memory_import_origin ON memory (created_by_ref)
    WHERE source = 'import';
