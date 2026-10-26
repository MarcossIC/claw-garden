-- V29: a project can be deleted, and a deleted one comes back when it is used.
--
-- The row of a deleted project is not removed: its deleted rows stay under its
-- id, `undelete` restores into it without any trigger adopting it, and undoing
-- the delete needs its name, its origin and its date. `deleted_at` marks it;
-- NULL for every project there is.
ALTER TABLE project ADD COLUMN deleted_at INTEGER;

-- One row per delete: the project, whether it took its rows with it, the code
-- of the set the user was shown, the snapshot written before, and when it was
-- undone or purged for good.
CREATE TABLE IF NOT EXISTS project_delete (
    delete_id     INTEGER PRIMARY KEY,
    project_id    TEXT    NOT NULL,
    with_memories INTEGER NOT NULL,
    code          TEXT,
    snapshot      TEXT,
    deleted_at    INTEGER NOT NULL,
    undone_at     INTEGER,
    purged_at     INTEGER
);

CREATE INDEX IF NOT EXISTS idx_project_delete_project ON project_delete (project_id);

-- Each row the delete took, with the `deleted_at` it had before; NULL means
-- the delete deleted it. The typed columns let an undo and a purge find their
-- rows by id through an index, which a generic text column would not allow.
CREATE TABLE IF NOT EXISTS project_delete_row (
    delete_id  INTEGER NOT NULL,
    tbl        TEXT    NOT NULL,
    row_id     INTEGER NOT NULL,
    deleted_at INTEGER
);

CREATE INDEX IF NOT EXISTS idx_project_delete_row ON project_delete_row (delete_id);

-- The folders whose docs index the delete turned off, so an undo turns on
-- only those.
CREATE TABLE IF NOT EXISTS project_delete_folder (
    delete_id INTEGER NOT NULL,
    root_id   TEXT    NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_project_delete_folder ON project_delete_folder (delete_id);

-- A live row under a deleted project's id means it is in use again, and it
-- comes back unreviewed, as an adopted one starts. The rule lives in triggers
-- on `memory`, as the adoption does, so it holds for every writer: a save, a
-- replay of the outbox, an import, a move and its undo. Registering a folder
-- or inserting an already-deleted row writes no live row and leaves it
-- deleted, so opening a session in the folder does not undo the delete.
DROP TRIGGER IF EXISTS memory_project_ai;
DROP TRIGGER IF EXISTS memory_project_au;

CREATE TRIGGER memory_project_ai AFTER INSERT ON memory
WHEN new.project_id IS NOT NULL BEGIN
    INSERT INTO project (id, name, born, reviewed, created_at)
    SELECT new.project_id,
           CASE WHEN new.project_id GLOB '?*-[0-9a-f][0-9a-f][0-9a-f][0-9a-f]'
                THEN substr(new.project_id, 1, length(new.project_id) - 5) ELSE new.project_id END,
           'adopted', 0, new.created_at
     WHERE NOT EXISTS (SELECT 1 FROM project WHERE id = new.project_id);
    UPDATE project SET deleted_at = NULL, reviewed = 0
     WHERE id = new.project_id AND deleted_at IS NOT NULL AND new.deleted_at IS NULL;
END;

CREATE TRIGGER memory_project_au AFTER UPDATE OF project_id ON memory
WHEN new.project_id IS NOT NULL BEGIN
    INSERT INTO project (id, name, born, reviewed, created_at)
    SELECT new.project_id,
           CASE WHEN new.project_id GLOB '?*-[0-9a-f][0-9a-f][0-9a-f][0-9a-f]'
                THEN substr(new.project_id, 1, length(new.project_id) - 5) ELSE new.project_id END,
           'adopted', 0, new.created_at
     WHERE NOT EXISTS (SELECT 1 FROM project WHERE id = new.project_id);
    UPDATE project SET deleted_at = NULL, reviewed = 0
     WHERE id = new.project_id AND deleted_at IS NOT NULL AND new.deleted_at IS NULL;
END;

-- A restore does not change the project of the row, so neither trigger above
-- sees it.
CREATE TRIGGER memory_project_restored AFTER UPDATE OF deleted_at ON memory
WHEN new.project_id IS NOT NULL AND old.deleted_at IS NOT NULL AND new.deleted_at IS NULL BEGIN
    UPDATE project SET deleted_at = NULL, reviewed = 0
     WHERE id = new.project_id AND deleted_at IS NOT NULL;
END;
