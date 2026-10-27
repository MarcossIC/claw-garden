-- V30: a subfolder of a repository declared as a project of its own.
--
-- A declaration says that, in the project `parent_id` of a repository, what
-- is worked on inside `subpath` (relative to the top of the checkout, its
-- components joined by `/`) goes to `project_id`. It keys on the project and
-- not on a remote, so a clone, a moved checkout without `origin`, a worktree
-- and the same store on another machine all reach it through the project
-- they already reach. Both ids are read through `merged_into`, so a merge
-- rewrites no declaration. `project_path` keeps holding roots alone.
CREATE TABLE IF NOT EXISTS project_subfolder (
    parent_id  TEXT    NOT NULL,
    subpath    TEXT    NOT NULL,
    project_id TEXT    NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (parent_id, subpath)
);

CREATE INDEX IF NOT EXISTS idx_project_subfolder_project ON project_subfolder (project_id);
