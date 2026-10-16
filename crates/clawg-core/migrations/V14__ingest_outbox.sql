-- V14: saves accepted but not yet stored.
--
-- A save is written here, already masked, before the daemon answers it, and
-- the row goes in the transaction that finishes storing it, so a process
-- that dies in between loses nothing. `content` is the masked text, kept in
-- its own column so a purge can find it; `event` is the rest of the event as
-- JSON, with an empty content. `progress` lists the pieces of a long save
-- already stored, written in the same transaction as each piece, so a replay
-- resumes after them. AUTOINCREMENT because the worker tells rows it already
-- took apart by id, and a reused id would read as one of them.
CREATE TABLE IF NOT EXISTS ingest_outbox (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    operation_id TEXT    NOT NULL,
    agent_id     TEXT    NOT NULL,
    content      TEXT    NOT NULL,
    event        TEXT    NOT NULL,
    state        TEXT    NOT NULL DEFAULT 'pending' CHECK (state IN ('pending', 'failed')),
    attempts     INTEGER NOT NULL DEFAULT 0,
    last_error   TEXT,
    progress     TEXT    NOT NULL DEFAULT '[]',
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL,
    UNIQUE (agent_id, operation_id)
);

CREATE INDEX IF NOT EXISTS idx_ingest_outbox_state ON ingest_outbox (state, id);
