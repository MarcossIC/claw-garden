-- V24: which project the clones of one remote share, and what joining them
-- replaced.
--
-- A root keeps its own id in `project`. A root cloned from a remote reads and
-- writes the memories of the project its remote names here: the id of the
-- first root that brought the remote. The key is the `origin` URL normalized
-- to host and path, never the URL itself, which can carry a password.
CREATE TABLE IF NOT EXISTS project_remote (
    remote      TEXT    PRIMARY KEY,
    project_id  TEXT    NOT NULL,
    created_at  INTEGER NOT NULL
);

-- Joining two projects moves rows (logged in `memory_move`) and then leaves one
-- live row per topic key and per text, superseding the others. Each of those
-- is logged with the state it had, so undoing move N is first
--   UPDATE memory SET status = s.from_status, superseded_by = s.from_superseded_by,
--          supersede_cause = s.from_cause
--     FROM memory_move_supersede AS s
--    WHERE s.move_id = N AND s.row_id = memory.id
--      AND memory.status = 'superseded' AND memory.superseded_by = s.superseded_by;
-- and then the move itself (V21).
CREATE TABLE IF NOT EXISTS memory_move_supersede (
    move_id            INTEGER NOT NULL,
    row_id             INTEGER NOT NULL,
    superseded_by      INTEGER NOT NULL,
    from_status        TEXT    NOT NULL,
    from_superseded_by INTEGER,
    from_cause         TEXT
);

CREATE INDEX IF NOT EXISTS idx_memory_move_supersede ON memory_move_supersede (move_id);

-- Finding the id a root was given.
CREATE INDEX IF NOT EXISTS idx_project_root ON project (root_path);
