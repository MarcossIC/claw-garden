-- V16: the topic a deliberate save is a version of.
--
-- A save that names a `topic_key` replaces the live rows its agent saved
-- under the same key in the same project: they become `superseded` by the
-- new save, so the history stays as rows. NULL for every row saved without
-- one, which is every row written before this column existed.
ALTER TABLE memory ADD COLUMN topic_key TEXT;

-- Not UNIQUE: the pieces of one long save share the key. One live save per
-- key is kept by the ingest worker, the single sequential writer of saves,
-- and by restore. COALESCE matches `idx_memory_dedup`, because project_id is
-- nullable and SQLite treats NULL as distinct from NULL.
CREATE INDEX IF NOT EXISTS idx_memory_topic
    ON memory (agent_id, COALESCE(project_id, ''), topic_key)
    WHERE deleted_at IS NULL AND topic_key IS NOT NULL;
