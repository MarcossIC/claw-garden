-- V10: where a memory came from, and when each scope was last reviewed.
--
-- A conclusion is a memory saved on top of others. One row per (conclusion,
-- source) pair says which memories it rests on; the conclusion and its
-- sources stay ordinary rows of `memory`, untouched. No foreign keys: rows of
-- `memory` are never deleted, only marked `deleted_at`, and a link to a
-- deleted row is history worth keeping.
CREATE TABLE IF NOT EXISTS memory_derivation (
    memory_id   INTEGER NOT NULL,
    source_id   INTEGER NOT NULL,
    created_at  INTEGER NOT NULL,
    PRIMARY KEY (memory_id, source_id)
) WITHOUT ROWID;

-- "Is this memory already behind a conclusion?" is asked by source.
CREATE INDEX IF NOT EXISTS idx_memory_derivation_source ON memory_derivation (source_id);

-- When an agent last looked at a scope for conclusions, so new memories can
-- be counted from there. Each agent reviews its own memories, so the review is
-- per agent: `agent_id` is '' for the store owner, who reviews every agent's.
-- `scope_key` is the project id, or '' for the whole store.
CREATE TABLE IF NOT EXISTS synthesis_review (
    agent_id     TEXT    NOT NULL,
    scope_key    TEXT    NOT NULL,
    reviewed_at  INTEGER NOT NULL,
    PRIMARY KEY (agent_id, scope_key)
) WITHOUT ROWID;
