-- V13: whether the daemon may read a project's files.
--
-- A root reaches `project` through any tool that names a path, so being
-- registered no longer means the user wants its files indexed. Rows that are
-- already here were registered by working in them, so they stay indexable.
ALTER TABLE project ADD COLUMN index_docs INTEGER NOT NULL DEFAULT 1;
