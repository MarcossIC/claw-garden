-- V27: a project is a row of its own, and a folder is one way to reach it.
--
-- `project` held folders: one row per repository a path of this machine held
-- (V9, V24), whose id is the docs index's key. Memories were never keyed by
-- it: a clone writes under the id of the first folder of its remote, and a
-- bridge whose registration failed under an id no folder has. So the table
-- of folders becomes `project_path`, values and indexes untouched, and
-- `project` is the reference every table keys its rows by, with or without
-- a folder or a remote.
ALTER TABLE project RENAME TO project_path;

-- The project a folder leads to (NULL until a registration records it: the
-- folder of a clone, whose project is its remote's, or one nothing was saved
-- in), and what was found there: the key of its remote, its first commit and
-- the mark of its git directory.
ALTER TABLE project_path ADD COLUMN project_id TEXT;
ALTER TABLE project_path ADD COLUMN remote TEXT;
ALTER TABLE project_path ADD COLUMN root_commit TEXT;
ALTER TABLE project_path ADD COLUMN git_stamp TEXT;

-- Finding the folders of one first commit, which a fork or a copy share: it
-- never links on its own, so it is not unique.
CREATE INDEX IF NOT EXISTS idx_project_path_root_commit ON project_path (root_commit)
    WHERE root_commit IS NOT NULL;

-- `born` has no CHECK: new ways for a project to start are coming, and a
-- CHECK cannot be widened without rebuilding the table. Rust validates it.
-- The name is not unique; the id tells two projects of one name apart.
CREATE TABLE IF NOT EXISTS project (
    id         TEXT    PRIMARY KEY,
    name       TEXT    NOT NULL,
    born       TEXT    NOT NULL,
    reviewed   INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL
);

-- Two projects that may be one, made when a registration could not tell:
-- once per pair whatever the order, so an answer keeps the pair from coming
-- back.
CREATE TABLE IF NOT EXISTS project_suggestion (
    project_id  TEXT    NOT NULL,
    other_id    TEXT    NOT NULL,
    reason      TEXT    NOT NULL,
    created_at  INTEGER NOT NULL,
    answer      TEXT,
    answered_at INTEGER
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_project_suggestion_pair
    ON project_suggestion (min(project_id, other_id), max(project_id, other_id));

-- A project for every id a row is saved under (deleted rows too) or a remote
-- names. An id only a log names (`memory_move`, `project_rekey`,
-- `synthesis_review`, `memory_import_extra`) gets none: it holds nothing to
-- show, and the triggers below give it one if a row comes back to it. A
-- folder's id alone is no project either: the folder of a clone keeps its
-- rows under its remote's. The name is the id without the `-` and four hex
-- digits `derive_project_id` appends; the date the oldest of what names it.
INSERT INTO project (id, name, born, reviewed, created_at)
SELECT id,
       CASE WHEN id GLOB '?*-[0-9a-f][0-9a-f][0-9a-f][0-9a-f]' THEN substr(id, 1, length(id) - 5) ELSE id END,
       'migrated', 1, MIN(created_at)
  FROM (SELECT project_id AS id, created_at, 1 AS used FROM memory WHERE project_id IS NOT NULL
        UNION ALL SELECT project_id, created_at, 1 FROM episodic_trajectory WHERE project_id IS NOT NULL
        UNION ALL SELECT project_id, created_at, 1 FROM procedural_skill WHERE project_id IS NOT NULL
        UNION ALL SELECT project_id, created_at, 1 FROM project_remote
        UNION ALL SELECT id, created_at, 0 FROM project_path)
 GROUP BY id
HAVING MAX(used) = 1;

UPDATE project_path SET project_id = id WHERE id IN (SELECT id FROM project);

-- A folder bound to one remote has found it there. One bound to two or more
-- was joined with another, and which of them is its own is not guessed.
UPDATE project_path
   SET remote = (SELECT remote FROM project_remote WHERE project_id = project_path.id)
 WHERE (SELECT COUNT(*) FROM project_remote WHERE project_id = project_path.id) = 1;

-- Every id written to `memory` from here on is a project: a save, an import,
-- a move and its undo, a PATCH, and a bridge that saves under an id it never
-- registered. Nothing is refused and the row is not touched, as the text
-- index's triggers (V8) keep a derived table in the same write. `NOT EXISTS`
-- rather than `INSERT OR IGNORE`: the conflict clause of the statement that
-- fires a trigger overrides the one inside it, so an `INSERT OR REPLACE INTO
-- memory` would replace the project.
CREATE TRIGGER IF NOT EXISTS memory_project_ai AFTER INSERT ON memory
WHEN new.project_id IS NOT NULL BEGIN
    INSERT INTO project (id, name, born, reviewed, created_at)
    SELECT new.project_id,
           CASE WHEN new.project_id GLOB '?*-[0-9a-f][0-9a-f][0-9a-f][0-9a-f]'
                THEN substr(new.project_id, 1, length(new.project_id) - 5) ELSE new.project_id END,
           'adopted', 0, new.created_at
     WHERE NOT EXISTS (SELECT 1 FROM project WHERE id = new.project_id);
END;

CREATE TRIGGER IF NOT EXISTS memory_project_au AFTER UPDATE OF project_id ON memory
WHEN new.project_id IS NOT NULL BEGIN
    INSERT INTO project (id, name, born, reviewed, created_at)
    SELECT new.project_id,
           CASE WHEN new.project_id GLOB '?*-[0-9a-f][0-9a-f][0-9a-f][0-9a-f]'
                THEN substr(new.project_id, 1, length(new.project_id) - 5) ELSE new.project_id END,
           'adopted', 0, new.created_at
     WHERE NOT EXISTS (SELECT 1 FROM project WHERE id = new.project_id);
END;
