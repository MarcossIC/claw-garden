-- V19: the pairs whose save was warned to take a topic key.
--
-- A save without a key that the detector paired with a row of its own agent
-- and project names the closest such row in `revision_candidate`, and the
-- bridge tells the agent to save it again under a key if it revises that row.
-- Doing so keys the save's own row, which then reads like a row saved under a
-- key from the start, so the warning is recorded on its pair when it fires.
-- The pairs written before this column were never warned about.
ALTER TABLE memory_conflict ADD COLUMN topic_key_warned INTEGER NOT NULL DEFAULT 0
    CHECK (topic_key_warned IN (0, 1));
