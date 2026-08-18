-- Lexical retrieval channel over `memory`. External-content (`content='memory'`)
-- rather than a copy: the indexed text is `memory.content` verbatim, so a second
-- physical copy could only ever disagree with the first.
--
-- `content_rowid='id'` is valid because `memory.id` is INTEGER PRIMARY KEY
-- AUTOINCREMENT, i.e. a true rowid alias. If that column's type ever changes,
-- this table silently stops resolving its content.
--
-- The three UNINDEXED columns are what let the candidate query apply the caller's
-- scope while the scan is still running. Without them the LIMIT is spent before
-- anything knows who is asking, so on a database holding several agents' rows the
-- whole candidate budget can be consumed by rows the scope join then discards —
-- a filter applied after the scan, which is exactly what retrieval here does not
-- do. UNINDEXED means the values are not tokenized: they are filter inputs, never
-- search terms, and a query for the literal text of a project id must not match
-- every row belonging to it. Their names MUST equal the column names on `memory`,
-- because an external-content table resolves values by selecting those names from
-- the content table.
--
-- They cannot go stale. An external-content table stores no column values at all;
-- FTS5 reads them from `memory` at query time, so `agent_id`, `project_id` and
-- `memory_type` are always the live values and no trigger has to maintain them.
--
-- `memory_type` is here for one reason: the candidate query copies the project
-- predicate from the KNN query byte for byte, and that predicate names
-- `memory_type = 'user_preference'`. Dropping the disjunct instead would make the
-- lexical channel stricter than the dense one about which project-scoped rows are
-- permitted, and nobody would find that until a user preference failed to surface
-- lexically while surfacing densely.
--
-- `unicode61 remove_diacritics 2` folds accents at index AND query time, so a
-- query typed `configuracion` reaches a row stored `configuración`. The corpus is
-- ES+EN, the same reason the embedder is multilingual. `porter` is rejected: it
-- stems English only and would leave the Spanish half of the corpus unstemmed
-- while damaging its tokens.
CREATE VIRTUAL TABLE IF NOT EXISTS memory_fts USING fts5(
    content,
    agent_id UNINDEXED,
    project_id UNINDEXED,
    memory_type UNINDEXED,
    content='memory',
    content_rowid='id',
    tokenize='unicode61 remove_diacritics 2'
);

-- Every live row is indexed exactly once and is subtracted exactly once, on the
-- first transition that removes it from the live set. That balance is the whole
-- correctness argument for the three triggers: on an external-content table the
-- removal form is a SUBTRACTIVE operation that assumes the exact tokens are
-- currently present, so a second subtraction of the same row corrupts the index,
-- and a subtraction with no matching addition corrupts it too.
--
-- The removal form takes a value for EVERY declared column, in declaration order.
-- The UNINDEXED ones contribute no tokens to subtract, but the argument list is
-- positional and must be complete — and passing the real `old.*` values keeps
-- these statements correct on the day one of those columns stops being UNINDEXED.
--
-- No guard on INSERT. A row inserted already carrying `deleted_at` is indexed like
-- any other, deliberately: guarding here would let the removal trigger below
-- subtract postings that were never added. Nothing leaks, because the scoring
-- query joins `memory` and re-applies `deleted_at IS NULL` — the index decides
-- which candidates are considered, never which rows a caller sees.
CREATE TRIGGER IF NOT EXISTS memory_fts_ai AFTER INSERT ON memory BEGIN
    INSERT INTO memory_fts(rowid, content, agent_id, project_id, memory_type)
    VALUES (new.id, new.content, new.agent_id, new.project_id, new.memory_type);
END;

-- Soft delete. `WHEN old.deleted_at IS NULL AND new.deleted_at IS NOT NULL`
-- restricts this to the live-to-deleted transition, so re-stamping `deleted_at` on
-- an already-deleted row cannot subtract a second time. The reverse transition has
-- no trigger because no code path un-deletes a memory; a trigger for an event that
-- cannot happen is a claim about the schema that is not true.
CREATE TRIGGER IF NOT EXISTS memory_fts_au_deleted
AFTER UPDATE OF deleted_at ON memory
WHEN old.deleted_at IS NULL AND new.deleted_at IS NOT NULL BEGIN
    INSERT INTO memory_fts(memory_fts, rowid, content, agent_id, project_id, memory_type)
    VALUES('delete', old.id, old.content, old.agent_id, old.project_id, old.memory_type);
END;

-- Hard delete. `WHEN old.deleted_at IS NULL` is what keeps the subtraction
-- exactly-once across both removal paths: a row that was soft-deleted first has
-- already been subtracted, and subtracting it again here would leave the index
-- counting postings it no longer holds.
CREATE TRIGGER IF NOT EXISTS memory_fts_ad AFTER DELETE ON memory
WHEN old.deleted_at IS NULL BEGIN
    INSERT INTO memory_fts(memory_fts, rowid, content, agent_id, project_id, memory_type)
    VALUES('delete', old.id, old.content, old.agent_id, old.project_id, old.memory_type);
END;

-- There is no trigger on `content`. It is write-once: no statement anywhere
-- updates it, and a correction creates a new row linked by `superseded_by`. The
-- day that stops being true, `content_hash` and the supersede model need a look as
-- well, and a trigger sitting here would hide that debt instead of surfacing it.
--
-- There is likewise no trigger on `agent_id`/`project_id`/`memory_type`: an
-- external-content table stores none of those values, so there is nothing that
-- could fall out of date with `memory`.

-- Backfill of rows written before this table existed. Filtered to live rows so the
-- index the migration produces is the same index the triggers maintain — one
-- invariant, not two that drift apart after the first delete.
INSERT INTO memory_fts(rowid, content, agent_id, project_id, memory_type)
    SELECT id, content, agent_id, project_id, memory_type
    FROM memory WHERE deleted_at IS NULL;
