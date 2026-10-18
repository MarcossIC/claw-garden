-- V15: the pairs the contradiction detector found, and how each was judged.
--
-- One row per pair of `memory` rows the detector saw close to each other.
-- `new_id` is the later row and `old_id` the earlier one. `distance` is the
-- L2 distance the detector measured, NULL for a pair that predates this
-- table. `state` is `pending` (only the agent was told), `escalated` (the
-- newer row names the older in `disputed_against` and the user is asked) or
-- `judged` (`verdict` says how it was closed). No foreign keys, as in
-- `memory_piece`: rows of `memory` are only removed by a purge, which removes
-- their pairs too.
CREATE TABLE IF NOT EXISTS memory_conflict (
    new_id      INTEGER NOT NULL,
    old_id      INTEGER NOT NULL,
    distance    REAL,
    state       TEXT    NOT NULL CHECK (state IN ('pending', 'escalated', 'judged')),
    verdict     TEXT    CHECK (verdict IN ('supersedes', 'compatible', 'scoped')),
    reason      TEXT,
    created_at  INTEGER NOT NULL,
    judged_at   INTEGER,
    PRIMARY KEY (new_id, old_id)
) WITHOUT ROWID;

-- "Which pairs name this row?" is asked of both sides.
CREATE INDEX IF NOT EXISTS idx_memory_conflict_old ON memory_conflict (old_id);
CREATE INDEX IF NOT EXISTS idx_memory_conflict_state ON memory_conflict (state);

-- Every dispute open today was shown to the user, so it stays escalated. Its
-- distance was never stored. A list that is not valid JSON names nothing, as
-- it does for every reader of the column.
INSERT OR IGNORE INTO memory_conflict (new_id, old_id, distance, state, created_at)
SELECT MAX(m.id, j.value), MIN(m.id, j.value), NULL, 'escalated',
       COALESCE(m.last_disputed_at, m.created_at)
FROM memory m, json_each(CASE WHEN json_valid(m.disputed_against) THEN m.disputed_against END) j
WHERE j.type = 'integer' AND j.value <> m.id;
