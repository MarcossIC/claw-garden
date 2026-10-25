-- V28: a project merged into another stays as a tombstone, and every merge
-- keeps what it changed so it can be undone.
--
-- The row of the project merged is not deleted: open bridges keep its id for
-- their whole session, old exports carry it, and without its row the next
-- save under it would adopt it again (V27's triggers) as an empty project.
-- `merged_into` names the live project every id of it leads to.
ALTER TABLE project ADD COLUMN merged_into TEXT;
ALTER TABLE project ADD COLUMN merged_at INTEGER;

CREATE INDEX IF NOT EXISTS idx_project_merged_into ON project (merged_into)
    WHERE merged_into IS NOT NULL;

-- One row per merge: the project merged, the one it went into, the move of
-- its rows of `memory` (`memory_move`, NULL when it held none), the snapshot
-- written before, and when it was undone, with the move that took its rows
-- back.
CREATE TABLE IF NOT EXISTS project_merge (
    merge_id     INTEGER PRIMARY KEY,
    source       TEXT    NOT NULL,
    target       TEXT    NOT NULL,
    move_id      INTEGER,
    snapshot     TEXT,
    merged_at    INTEGER NOT NULL,
    undone_at    INTEGER,
    undo_move_id INTEGER
);

-- Everything else a merge changed, with what it was before, as
-- `memory_import_extra` (V26) keeps an import's. `kind` has no CHECK, so a
-- new kind needs no rebuild; Rust reads it.
CREATE TABLE IF NOT EXISTS project_merge_change (
    merge_id INTEGER NOT NULL,
    kind     TEXT    NOT NULL,
    a        TEXT    NOT NULL,
    b        TEXT
);

CREATE INDEX IF NOT EXISTS idx_project_merge_change ON project_merge_change (merge_id);
