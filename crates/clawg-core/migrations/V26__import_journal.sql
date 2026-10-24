-- V26: what each import wrote, so it can be taken back without losing what
-- was saved after it.
--
-- The `pre-import` snapshot gives the whole store back as it was, and with it
-- discards every later save. This journal names, import by import, the rows
-- it inserted and what it changed in rows that were already here, each with
-- the value it left, so undoing import N takes back only what is still as N
-- left it. Written in the transaction of each write of the import.
-- No foreign key: `purge` removes rows for good, and an old import must not
-- stop it.

-- One row per import that wrote anything. `finished_at` stays NULL when the
-- process died before the links were written: that import is undone as any.
CREATE TABLE IF NOT EXISTS memory_import (
    import_id   INTEGER PRIMARY KEY,
    started_at  INTEGER NOT NULL,
    finished_at INTEGER,
    source      TEXT    NOT NULL,
    snapshot    TEXT
);

-- Every row the import inserted, with the columns only an edit, a delete, a
-- restore or a verdict change afterwards. Maintenance and a reinforcing save
-- move the tier, the accesses and `last_modified_at`, so those are no sign.
CREATE TABLE IF NOT EXISTS memory_import_row (
    import_id       INTEGER NOT NULL,
    row_id          INTEGER NOT NULL,
    update_count    INTEGER NOT NULL,
    deleted_at      INTEGER,
    supersede_cause TEXT
);

CREATE INDEX IF NOT EXISTS idx_memory_import_row ON memory_import_row (import_id, row_id);

-- A row of an earlier import that a newer version of its note replaced, with
-- the state it had, as `memory_move_supersede` (V24) logs a join.
CREATE TABLE IF NOT EXISTS memory_import_supersede (
    import_id          INTEGER NOT NULL,
    row_id             INTEGER NOT NULL,
    superseded_by      INTEGER NOT NULL,
    from_status        TEXT    NOT NULL,
    from_superseded_by INTEGER,
    from_cause         TEXT
);

CREATE INDEX IF NOT EXISTS idx_memory_import_supersede ON memory_import_supersede (import_id);

-- What the import added that hangs from rows it did not insert, and does not
-- go with the rows it did:
--   fill        a = row, b = column (superseded_by, revived_from,
--               disputed_against), value = what was written where it was NULL
--   pair        a = new_id, b = old_id, value = [state, verdict] as written
--   derivation  a = memory_id, b = source_id
--   piece       a = save_id, b = piece
--   project     b = the root id
--   remote      b = the remote
CREATE TABLE IF NOT EXISTS memory_import_extra (
    import_id INTEGER NOT NULL,
    kind      TEXT    NOT NULL
                      CHECK (kind IN ('fill', 'pair', 'derivation', 'piece', 'project', 'remote')),
    a         INTEGER,
    b         TEXT,
    value     TEXT
);

CREATE INDEX IF NOT EXISTS idx_memory_import_extra ON memory_import_extra (import_id);
