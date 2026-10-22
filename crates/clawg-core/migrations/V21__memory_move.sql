-- V21: which memory rows were moved to another project, and from where.
--
-- A move changes a row's project_id, and its scope when the CHECK of `memory`
-- needs it (a row no agent saved is 'project' with a project and 'global'
-- without one). Neither project may exist, which `project_rekey` cannot say,
-- so a move keeps a log of its own. Undoing move N is
--   UPDATE memory SET project_id = m.from_project, scope = m.from_scope
--     FROM memory_move AS m
--    WHERE m.move_id = N AND m.row_id = memory.id AND memory.project_id IS m.to_project;
-- No foreign key: `purge` removes rows for good, and an old move must not stop it.
CREATE TABLE IF NOT EXISTS memory_move (
    move_id      INTEGER NOT NULL,
    row_id       INTEGER NOT NULL,
    from_project TEXT,
    from_scope   TEXT    NOT NULL,
    to_project   TEXT,
    to_scope     TEXT    NOT NULL,
    moved_at     INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_memory_move ON memory_move (move_id);
