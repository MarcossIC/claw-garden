-- V20: the dedup and the topic key compare a save with its project.
--
-- A save reinforces the live row of its text, and replaces the live rows of
-- its key, in its project whoever saved them; the author is only a residual
-- filter for what `sharing.json` hides from the saver. Both indexes led with
-- `agent_id`, which no lookup binds any more, so they start at the project.
-- No row changes.
DROP INDEX IF EXISTS idx_memory_dedup;
CREATE INDEX IF NOT EXISTS idx_memory_dedup
    ON memory (COALESCE(project_id, ''), content_hash, created_at)
    WHERE deleted_at IS NULL AND content_hash IS NOT NULL;

DROP INDEX IF EXISTS idx_memory_topic;
CREATE INDEX IF NOT EXISTS idx_memory_topic
    ON memory (COALESCE(project_id, ''), topic_key)
    WHERE deleted_at IS NULL AND topic_key IS NOT NULL;
