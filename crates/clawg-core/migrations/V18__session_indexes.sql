-- V18: the newest rows LAST SESSION and RECENT read on every context.
--
-- Both order by `created_at DESC, id ASC`. Equal keys sit in an index by rowid
-- ascending, and `id` is the rowid, so walking `created_at DESC` yields the
-- tie-break without a sort; an ASC index would still sort each stamp's rows.
-- `status` stays out of the predicates: a partial index serves a statement
-- only when its WHERE implies the index's, which would tie these to the
-- statements' wording of the live states.
CREATE INDEX IF NOT EXISTS idx_memory_session_summary
    ON memory (project_id, created_at DESC)
    WHERE memory_type = 'session_summary' AND deleted_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_memory_agent_created
    ON memory (agent_id, created_at DESC)
    WHERE deleted_at IS NULL;

-- The dedup lookups (a live row of the same text, oldest first) also order by
-- `created_at` under `agent_id = ?` with `LIMIT 1`, so the index above would
-- read every live row of the agent for them. With the stamp after the hash,
-- the dedup index answers both their equality and their order.
DROP INDEX IF EXISTS idx_memory_dedup;
CREATE INDEX IF NOT EXISTS idx_memory_dedup
    ON memory (agent_id, COALESCE(project_id, ''), content_hash, created_at)
    WHERE deleted_at IS NULL AND content_hash IS NOT NULL;
