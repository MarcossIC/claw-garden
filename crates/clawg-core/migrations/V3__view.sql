-- V3: all_memories UNION ALL view.
-- Used by Phase 4+ retrieval to scan all memory kinds in a single query
-- when scope/agent_id are the only filters.
-- The `kind` discriminator column identifies the source table.

CREATE VIEW IF NOT EXISTS all_memories AS
SELECT id, scope, agent_id, project_id, content AS body,
       'memory' AS kind, created_at, last_accessed_at, importance, tier,
       status, dispute_count, deleted_at
FROM memory
UNION ALL
SELECT id, scope, agent_id, project_id, task AS body,
       'episodic' AS kind, created_at, last_accessed_at, importance, tier,
       status, dispute_count, deleted_at
FROM episodic_trajectory
UNION ALL
SELECT id, scope, agent_id, project_id, skill_name AS body,
       'procedural' AS kind, created_at, last_accessed_at, importance, tier,
       status, dispute_count, deleted_at
FROM procedural_skill;
