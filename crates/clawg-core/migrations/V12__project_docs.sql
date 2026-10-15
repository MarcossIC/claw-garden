-- V11: an index of each registered project's documentation.
--
-- Everything here is derived from files on disk and can be rebuilt from them,
-- so none of it is memory: no tiers, no decay, no disputes, and no statement
-- that reads or writes these tables touches `memory` or its indexes.

-- What each indexed file looked like when its passages were written. A file
-- whose bytes and model still match is not re-read into passages; a model
-- change reindexes it, because its vectors belong to the old model's space.
CREATE TABLE IF NOT EXISTS project_doc_file (
    project_id   TEXT    NOT NULL,
    path         TEXT    NOT NULL,
    content_hash TEXT    NOT NULL,
    model_id     TEXT    NOT NULL,
    chunks       INTEGER NOT NULL,
    indexed_at   INTEGER NOT NULL,
    PRIMARY KEY (project_id, path)
) WITHOUT ROWID;

-- One row per passage: a section of a file, or a piece of a long one.
-- `id` is the rowid of its vector and of its full-text entry.
CREATE TABLE IF NOT EXISTS project_doc_chunk (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id  TEXT    NOT NULL,
    path        TEXT    NOT NULL,
    ordinal     INTEGER NOT NULL,
    breadcrumb  TEXT    NOT NULL,
    body        TEXT    NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_project_doc_chunk_file ON project_doc_chunk (project_id, path);

-- `project_id` as a partition key puts the project filter inside the KNN scan,
-- so k neighbours are k neighbours of that project and a large repository
-- cannot crowd a small one out of its own results.
CREATE VIRTUAL TABLE IF NOT EXISTS project_doc_vectors USING vec0(
    project_id TEXT PARTITION KEY,
    embedding float[384]
);

-- A table holding its own text rather than external content: a passage is
-- replaced whole on every reindex, so a DELETE and an INSERT in the same
-- transaction keep it exact without the triggers `memory_fts` needs. Same
-- tokenizer as `memory_fts`, for the same mixed Spanish and English text.
CREATE VIRTUAL TABLE IF NOT EXISTS project_doc_fts USING fts5(
    text,
    project_id UNINDEXED,
    tokenize = 'unicode61 remove_diacritics 2'
);
