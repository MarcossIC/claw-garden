-- V31: what an import adds to the registry of projects, so its undo can take
-- it back, and an index that finds a row by its date in any project.
--
-- `memory_import_extra` (V26) keeps its CHECK and its kinds: `project` there
-- is still a folder the import recorded and `remote` a remote it bound. This
-- journal names the projects an import created or brought back from a
-- delete, the suggestions it left and the subfolders it declared. Its
-- columns are text because what it names are project ids and subpaths;
-- `kind` has no CHECK, so a new kind needs no rebuild; Rust reads it.
CREATE TABLE IF NOT EXISTS memory_import_registry (
    import_id INTEGER NOT NULL,
    kind      TEXT    NOT NULL,
    a         TEXT,
    b         TEXT,
    value     TEXT
);

CREATE INDEX IF NOT EXISTS idx_memory_import_registry ON memory_import_registry (import_id);

-- A row imported again after it was moved is found wherever it is now, by its
-- date, deleted or not, so the index has no predicate.
CREATE INDEX IF NOT EXISTS idx_memory_created_at ON memory (created_at);
