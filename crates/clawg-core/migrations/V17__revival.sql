-- V17: why a row was superseded, and which delete brought one back.
--
-- `supersede_cause` is written with `superseded_by`: 'revision' when a save
-- under the same topic key replaced the row, 'verdict' when a `supersedes`
-- verdict did. Deleting the current version of a topic brings back only the
-- version a revision replaced, never the loser of a verdict, so the cause has
-- to be recorded rather than read off the keys: undoing a revision with a
-- verdict leaves two rows of one key linked by a verdict.
ALTER TABLE memory ADD COLUMN supersede_cause TEXT CHECK (supersede_cause IN ('revision', 'verdict'));

-- The deleted row whose delete brought this one back. Restoring that row
-- supersedes this one again, and only this one: any other live row of the
-- key still keeps it deleted.
ALTER TABLE memory ADD COLUMN revived_from INTEGER;

CREATE INDEX IF NOT EXISTS idx_memory_revived
    ON memory (revived_from)
    WHERE revived_from IS NOT NULL;

-- Before V16 only a verdict superseded a row. Since V16 a revision writes no
-- pair, so a judged `supersedes` pair between the two rows means a verdict
-- even when they share the key.
UPDATE memory SET supersede_cause = CASE
    WHEN memory.topic_key IS NOT NULL AND EXISTS (
        SELECT 1 FROM memory s
        WHERE s.id = memory.superseded_by
          AND s.agent_id IS memory.agent_id
          AND COALESCE(s.project_id, '') = COALESCE(memory.project_id, '')
          AND s.topic_key = memory.topic_key)
     AND NOT EXISTS (
        SELECT 1 FROM memory_conflict p
        WHERE p.new_id = MAX(memory.id, memory.superseded_by)
          AND p.old_id = MIN(memory.id, memory.superseded_by)
          AND p.verdict = 'supersedes')
    THEN 'revision'
    ELSE 'verdict'
END
WHERE status = 'superseded' AND superseded_by IS NOT NULL;
