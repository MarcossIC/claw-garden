-- V25: the dedup index holds every row.
--
-- An import asks whether a row of an agent, a project, a text and a date is
-- here, deleted or not, so its lookup carries no `deleted_at` term, and a
-- partial index serves a statement only when the statement's WHERE implies
-- the index's: it scanned `memory` once per row of the file. Without the
-- predicate the same columns serve it, and the lookups of a live text still
-- reach their rows by key and leave the deleted ones as a residual filter.
-- No row changes.
DROP INDEX IF EXISTS idx_memory_dedup;
CREATE INDEX IF NOT EXISTS idx_memory_dedup
    ON memory (COALESCE(project_id, ''), content_hash, created_at);
