-- V32: the rows of `episodic_trajectory` and `procedural_skill` move to
-- `memory`, and the two tables go with their vectors.
--
-- Nothing has written either table since the desktop app that did went away,
-- yet every search, maintenance pass and project operation still read them.
-- No row is lost: each one, live or deleted, becomes a `semantic_fact` of
-- `memory` whose `source` is the table it came from, the columns `memory`
-- lacks written into its text one line each, and every id that named it
-- rewritten to its new one. Undoing this is restoring the `pre-migration`
-- snapshot written before it runs.

-- Where each row went: the only record of the ids a row had before. Nothing
-- reads it, as nothing reads `project_rekey` (V9).
CREATE TABLE IF NOT EXISTS legacy_kind_move (
    new_id INTEGER PRIMARY KEY,
    tbl    TEXT    NOT NULL,
    old_id INTEGER NOT NULL,
    UNIQUE (tbl, old_id)
);

-- From past the largest id `memory` ever gave, so a purged id is not given
-- again; an explicit id above it moves `sqlite_sequence` on.
INSERT INTO legacy_kind_move (new_id, tbl, old_id)
SELECT MAX(COALESCE((SELECT seq FROM sqlite_sequence WHERE name = 'memory'), 0),
           COALESCE((SELECT MAX(id) FROM memory), 0))
       + ROW_NUMBER() OVER (ORDER BY ord, id),
       tbl, id
  FROM (SELECT 1 AS ord, 'episodic_trajectory' AS tbl, id FROM episodic_trajectory
        UNION ALL
        SELECT 2, 'procedural_skill', id FROM procedural_skill);

-- Every row goes in live and the deleted ones are deleted after, as an
-- import does: the text index trigger of V8 indexes every insert, and a
-- restore indexes a row again, so a row inserted deleted would end up
-- indexed twice. A live insert under a deleted project brings it back
-- (V29), so the deleted projects are put back as they were.
CREATE TEMP TABLE legacy_kind_project AS
    SELECT id, deleted_at, reviewed FROM project WHERE deleted_at IS NOT NULL;

-- A trajectory's `session_id` has a column of its own and goes to the text
-- too: it is all that tells two trajectories of one task apart, which would
-- otherwise be two live rows of one project with one `content_hash`. An id
-- that names no row of its table would name some unrelated row of `memory`
-- under its old number, so it is dropped; a list that is not valid JSON
-- names nothing and is kept as it is.
INSERT INTO memory (id, scope, agent_id, project_id, content, memory_type, source, created_at, created_by_type,
                    created_by_ref, source_session_id, last_modified_at, last_modified_by, update_count,
                    last_accessed_at, access_count, importance, tier, superseded_by, status, dispute_count,
                    disputed_against, last_disputed_at, supersede_cause)
SELECT m.new_id, t.scope, t.agent_id, t.project_id,
       t.task
         || CASE WHEN t.resolution IS NOT NULL THEN char(10) || 'resolution: ' || t.resolution ELSE '' END
         || CASE WHEN t.root_cause IS NOT NULL THEN char(10) || 'root_cause: ' || t.root_cause ELSE '' END
         || char(10) || 'tool_calls: ' || t.tool_calls
         || char(10) || 'errors_encountered: ' || t.errors_encountered
         || char(10) || 'success_score: ' || t.success_score
         || char(10) || 'session_id: ' || t.session_id,
       'semantic_fact', 'episodic_trajectory', t.created_at, t.created_by_type, t.created_by_ref, t.session_id,
       t.last_modified_at, t.last_modified_by, t.update_count, t.last_accessed_at, t.access_count, t.importance,
       t.tier, s.new_id, t.status, t.dispute_count,
       CASE WHEN json_valid(t.disputed_against) THEN
           (SELECT json_group_array(d.new_id) FROM json_each(t.disputed_against) j
              JOIN legacy_kind_move d ON d.tbl = 'episodic_trajectory' AND d.old_id = j.value
             WHERE j.type = 'integer')
       ELSE t.disputed_against END,
       t.last_disputed_at,
       CASE WHEN t.status = 'superseded' AND s.new_id IS NOT NULL THEN 'verdict' END
  FROM episodic_trajectory t
  JOIN legacy_kind_move m ON m.tbl = 'episodic_trajectory' AND m.old_id = t.id
  LEFT JOIN legacy_kind_move s ON s.tbl = 'episodic_trajectory' AND s.old_id = t.superseded_by
 ORDER BY m.new_id;

INSERT INTO memory (id, scope, agent_id, project_id, content, memory_type, source, created_at, created_by_type,
                    created_by_ref, last_modified_at, last_modified_by, update_count, last_accessed_at,
                    access_count, importance, tier, superseded_by, status, dispute_count, disputed_against,
                    last_disputed_at, supersede_cause)
SELECT m.new_id, p.scope, p.agent_id, p.project_id,
       p.skill_name
         || char(10) || 'trigger: ' || p.trigger
         || char(10) || 'tool_sequence: ' || p.tool_sequence
         || char(10) || 'success_count: ' || p.success_count
         || char(10) || 'promoted_at: ' || p.promoted_at
         || char(10) || 'source_trajectory_ids: '
         || CASE WHEN json_valid(p.source_trajectory_ids) THEN
                (SELECT json_group_array(d.new_id) FROM json_each(p.source_trajectory_ids) j
                   JOIN legacy_kind_move d ON d.tbl = 'episodic_trajectory' AND d.old_id = j.value
                  WHERE j.type = 'integer')
            ELSE p.source_trajectory_ids END,
       'semantic_fact', 'procedural_skill', p.created_at, p.created_by_type, p.created_by_ref,
       p.last_modified_at, p.last_modified_by, p.update_count, p.last_accessed_at, p.access_count, p.importance,
       p.tier, s.new_id, p.status, p.dispute_count,
       CASE WHEN json_valid(p.disputed_against) THEN
           (SELECT json_group_array(d.new_id) FROM json_each(p.disputed_against) j
              JOIN legacy_kind_move d ON d.tbl = 'procedural_skill' AND d.old_id = j.value
             WHERE j.type = 'integer')
       ELSE p.disputed_against END,
       p.last_disputed_at,
       CASE WHEN p.status = 'superseded' AND s.new_id IS NOT NULL THEN 'verdict' END
  FROM procedural_skill p
  JOIN legacy_kind_move m ON m.tbl = 'procedural_skill' AND m.old_id = p.id
  LEFT JOIN legacy_kind_move s ON s.tbl = 'procedural_skill' AND s.old_id = p.superseded_by
 ORDER BY m.new_id;

UPDATE memory SET content_hash = content_hash(content) WHERE id IN (SELECT new_id FROM legacy_kind_move);

-- The vector of a live row comes with it. A deleted row gets one when it is
-- restored, as any deleted row of `memory` does.
INSERT INTO memory_vectors (rowid, embedding)
SELECT m.new_id, v.embedding
  FROM legacy_kind_move m
  JOIN episodic_trajectory t ON t.id = m.old_id
  JOIN episodic_vectors v ON v.rowid = t.id
 WHERE m.tbl = 'episodic_trajectory' AND t.deleted_at IS NULL;

INSERT INTO memory_vectors (rowid, embedding)
SELECT m.new_id, v.embedding
  FROM legacy_kind_move m
  JOIN procedural_skill p ON p.id = m.old_id
  JOIN procedural_vectors v ON v.rowid = p.id
 WHERE m.tbl = 'procedural_skill' AND p.deleted_at IS NULL;

UPDATE memory SET deleted_at = o.deleted_at
  FROM (SELECT m.new_id, t.deleted_at FROM legacy_kind_move m
          JOIN episodic_trajectory t ON m.tbl = 'episodic_trajectory' AND t.id = m.old_id
        UNION ALL
        SELECT m.new_id, p.deleted_at FROM legacy_kind_move m
          JOIN procedural_skill p ON m.tbl = 'procedural_skill' AND p.id = m.old_id) AS o
 WHERE memory.id = o.new_id AND o.deleted_at IS NOT NULL;

UPDATE project SET deleted_at = d.deleted_at, reviewed = d.reviewed
  FROM temp.legacy_kind_project AS d
 WHERE project.id = d.id AND project.deleted_at IS NULL;

-- Every dispute between moved rows is an escalated pair, as V15 made of
-- every dispute open then.
INSERT OR IGNORE INTO memory_conflict (new_id, old_id, distance, state, created_at)
SELECT MAX(r.id, j.value), MIN(r.id, j.value), NULL, 'escalated', COALESCE(r.last_disputed_at, r.created_at)
  FROM memory r
  JOIN legacy_kind_move m ON m.new_id = r.id,
       json_each(CASE WHEN json_valid(r.disputed_against) THEN r.disputed_against END) j
 WHERE j.type = 'integer' AND j.value <> r.id;

-- The journals name the rows by their new ids. A delete and the registry's
-- rekey already read `memory`; a merge took its rows back by table, so each
-- of them becomes a row of the merge's move, which its undo takes back as
-- every other row of `memory`. A merge that moved no row of `memory` gets
-- the next free move. An entry naming a row that is no longer there (a
-- purge removed it) has nowhere to go and stays.
UPDATE project_delete_row SET tbl = 'memory', row_id = m.new_id
  FROM legacy_kind_move AS m
 WHERE m.tbl = project_delete_row.tbl AND m.old_id = project_delete_row.row_id;

UPDATE project_rekey SET tbl = 'memory', row_id = m.new_id
  FROM legacy_kind_move AS m
 WHERE m.tbl = project_rekey.tbl AND m.old_id = project_rekey.row_id;

CREATE TEMP TABLE legacy_kind_merged AS
    SELECT c.rowid AS change, c.merge_id, m.new_id
      FROM project_merge_change c
      JOIN legacy_kind_move m
        ON m.tbl = CASE c.kind WHEN 'trajectory' THEN 'episodic_trajectory' ELSE 'procedural_skill' END
       AND m.old_id = CAST(c.a AS INTEGER)
     WHERE c.kind IN ('trajectory', 'skill');

UPDATE project_merge
   SET move_id = (SELECT COALESCE(MAX(move_id), 0) FROM memory_move) + n.n
  FROM (SELECT merge_id, ROW_NUMBER() OVER (ORDER BY merge_id) AS n
          FROM project_merge
         WHERE move_id IS NULL AND merge_id IN (SELECT merge_id FROM temp.legacy_kind_merged)) AS n
 WHERE project_merge.merge_id = n.merge_id;

INSERT INTO memory_move (move_id, row_id, from_project, from_scope, to_project, to_scope, moved_at)
SELECT p.move_id, g.new_id, p.source, r.scope, p.target, r.scope, p.merged_at
  FROM temp.legacy_kind_merged g
  JOIN project_merge p ON p.merge_id = g.merge_id
  JOIN memory r ON r.id = g.new_id
 ORDER BY g.change;

DELETE FROM project_merge_change WHERE rowid IN (SELECT change FROM temp.legacy_kind_merged);

DROP TABLE temp.legacy_kind_merged;
DROP TABLE temp.legacy_kind_project;
DROP TABLE episodic_vectors;
DROP TABLE procedural_vectors;
DROP TABLE episodic_trajectory;
DROP TABLE procedural_skill;
