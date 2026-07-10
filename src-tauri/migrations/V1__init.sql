-- V1: Core memory tables — memory, episodic_trajectory, procedural_skill
-- All tables include scope/agent_id/project_id CHECK constraints and partial indices.

CREATE TABLE IF NOT EXISTS memory (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    scope               TEXT    NOT NULL DEFAULT 'agent'
                                CHECK (scope IN ('agent', 'project', 'global')),
    agent_id            TEXT,
    project_id          TEXT,
    content             TEXT    NOT NULL,
    memory_type         TEXT    NOT NULL DEFAULT 'observation',
    source              TEXT    NOT NULL DEFAULT 'agent',
    created_at          INTEGER NOT NULL,
    created_by_type     TEXT    NOT NULL DEFAULT 'agent',
    created_by_ref      TEXT    NOT NULL DEFAULT '',
    source_session_id   TEXT,
    last_modified_at    INTEGER NOT NULL,
    last_modified_by    TEXT,
    update_count        INTEGER NOT NULL DEFAULT 0,
    last_accessed_at    INTEGER NOT NULL,
    access_count        INTEGER NOT NULL DEFAULT 0,
    importance          REAL    NOT NULL DEFAULT 5.0,
    tier                INTEGER NOT NULL DEFAULT 3,
    superseded_by       INTEGER,
    status              TEXT    NOT NULL DEFAULT 'active',
    dispute_count       INTEGER NOT NULL DEFAULT 0,
    disputed_against    TEXT,
    last_disputed_at    INTEGER,
    deleted_at          INTEGER,
    CHECK (
        (scope = 'agent'   AND agent_id   IS NOT NULL) OR
        (scope = 'project' AND project_id IS NOT NULL AND agent_id IS NULL) OR
        (scope = 'global'  AND agent_id   IS NULL     AND project_id IS NULL)
    )
);

CREATE INDEX IF NOT EXISTS idx_memory_agent_active
    ON memory (agent_id, last_accessed_at)
    WHERE deleted_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_memory_scope_project
    ON memory (scope, project_id)
    WHERE deleted_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_memory_last_accessed
    ON memory (last_accessed_at);

CREATE INDEX IF NOT EXISTS idx_memory_superseded
    ON memory (superseded_by)
    WHERE superseded_by IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_memory_disputed
    ON memory (status)
    WHERE status = 'disputed';

CREATE INDEX IF NOT EXISTS idx_memory_tier_status
    ON memory (tier, status);

-- ---------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS episodic_trajectory (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    scope               TEXT    NOT NULL DEFAULT 'agent'
                                CHECK (scope IN ('agent', 'project', 'global')),
    agent_id            TEXT,
    project_id          TEXT,
    session_id          TEXT    NOT NULL UNIQUE,
    task                TEXT    NOT NULL,
    tool_calls          TEXT    NOT NULL DEFAULT '[]',
    errors_encountered  TEXT    NOT NULL DEFAULT '[]',
    resolution          TEXT,
    success_score       REAL    NOT NULL DEFAULT 0.0,
    created_at          INTEGER NOT NULL,
    created_by_type     TEXT    NOT NULL DEFAULT 'agent',
    created_by_ref      TEXT    NOT NULL DEFAULT '',
    last_modified_at    INTEGER NOT NULL,
    last_modified_by    TEXT,
    update_count        INTEGER NOT NULL DEFAULT 0,
    last_accessed_at    INTEGER NOT NULL,
    access_count        INTEGER NOT NULL DEFAULT 0,
    importance          REAL    NOT NULL DEFAULT 5.0,
    tier                INTEGER NOT NULL DEFAULT 3,
    superseded_by       INTEGER,
    status              TEXT    NOT NULL DEFAULT 'active',
    dispute_count       INTEGER NOT NULL DEFAULT 0,
    disputed_against    TEXT,
    last_disputed_at    INTEGER,
    deleted_at          INTEGER,
    CHECK (
        (scope = 'agent'   AND agent_id   IS NOT NULL) OR
        (scope = 'project' AND project_id IS NOT NULL AND agent_id IS NULL) OR
        (scope = 'global'  AND agent_id   IS NULL     AND project_id IS NULL)
    )
);

CREATE INDEX IF NOT EXISTS idx_epi_agent_active
    ON episodic_trajectory (agent_id, last_accessed_at)
    WHERE deleted_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_epi_scope_project
    ON episodic_trajectory (scope, project_id)
    WHERE deleted_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_epi_last_accessed
    ON episodic_trajectory (last_accessed_at);

CREATE INDEX IF NOT EXISTS idx_epi_tier_status
    ON episodic_trajectory (tier, status);

-- ---------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS procedural_skill (
    id                      INTEGER PRIMARY KEY AUTOINCREMENT,
    scope                   TEXT    NOT NULL DEFAULT 'agent'
                                    CHECK (scope IN ('agent', 'project', 'global')),
    agent_id                TEXT,
    project_id              TEXT,
    skill_name              TEXT    NOT NULL,
    trigger                 TEXT    NOT NULL,
    tool_sequence           TEXT    NOT NULL DEFAULT '[]',
    success_count           INTEGER NOT NULL DEFAULT 0,
    promoted_at             INTEGER NOT NULL,
    source_trajectory_ids   TEXT    NOT NULL,
    created_at              INTEGER NOT NULL,
    created_by_type         TEXT    NOT NULL DEFAULT 'agent',
    created_by_ref          TEXT    NOT NULL DEFAULT '',
    last_modified_at        INTEGER NOT NULL,
    last_modified_by        TEXT,
    update_count            INTEGER NOT NULL DEFAULT 0,
    last_accessed_at        INTEGER NOT NULL,
    access_count            INTEGER NOT NULL DEFAULT 0,
    importance              REAL    NOT NULL DEFAULT 8.0,
    tier                    INTEGER NOT NULL DEFAULT 1,
    superseded_by           INTEGER,
    status                  TEXT    NOT NULL DEFAULT 'active',
    dispute_count           INTEGER NOT NULL DEFAULT 0,
    disputed_against        TEXT,
    last_disputed_at        INTEGER,
    deleted_at              INTEGER,
    CHECK (
        (scope = 'agent'   AND agent_id   IS NOT NULL) OR
        (scope = 'project' AND project_id IS NOT NULL AND agent_id IS NULL) OR
        (scope = 'global'  AND agent_id   IS NULL     AND project_id IS NULL)
    )
);

CREATE INDEX IF NOT EXISTS idx_proc_agent_active
    ON procedural_skill (agent_id, last_accessed_at)
    WHERE deleted_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_proc_scope_project
    ON procedural_skill (scope, project_id)
    WHERE deleted_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_proc_last_accessed
    ON procedural_skill (last_accessed_at);

CREATE INDEX IF NOT EXISTS idx_proc_tier_status
    ON procedural_skill (tier, status);
