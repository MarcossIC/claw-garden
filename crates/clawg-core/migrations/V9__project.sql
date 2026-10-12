-- V9: where each project lives, and which rows were moved to it.
--
-- A project_id is a hash of a path, and a hash cannot be turned back into the
-- path it came from. `project` keeps the canonical root next to its id, so a
-- future change to how ids are derived can recompute them instead of guessing.
CREATE TABLE IF NOT EXISTS project (
    id          TEXT    PRIMARY KEY,
    root_path   TEXT    NOT NULL,
    created_at  INTEGER NOT NULL
);

-- One row per stored row whose project_id was rewritten from a legacy id. Several
-- legacy ids can land on one project, so only a per-row record says where each
-- row came from; restoring them is
--   UPDATE <tbl> SET project_id = r.legacy_id FROM project_rekey AS r
--    WHERE r.tbl = '<tbl>' AND r.row_id = <tbl>.id AND r.project_id = <tbl>.project_id;
CREATE TABLE IF NOT EXISTS project_rekey (
    tbl         TEXT    NOT NULL
                        CHECK (tbl IN ('memory', 'episodic_trajectory', 'procedural_skill')),
    row_id      INTEGER NOT NULL,
    legacy_id   TEXT    NOT NULL,
    project_id  TEXT    NOT NULL,
    rekeyed_at  INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_project_rekey_row ON project_rekey (tbl, row_id);
