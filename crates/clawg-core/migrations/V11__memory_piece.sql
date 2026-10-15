-- V11: which rows hold the pieces of one long save.
--
-- A text longer than one piece is stored as several `memory` rows. One row
-- here per piece says which save it belongs to and where it goes; the rows
-- themselves stay ordinary memories, untouched. `context` is the opening of
-- the save the row's vector was embedded behind, or '' when the row was not
-- embedded with one (the first piece, or a row an earlier save already held),
-- so a re-embed can rebuild the same vector. No foreign keys: rows of
-- `memory` are never deleted, only marked `deleted_at`, and a link to a
-- deleted row says which piece is missing.
CREATE TABLE IF NOT EXISTS memory_piece (
    save_id    INTEGER NOT NULL,
    piece      INTEGER NOT NULL,
    memory_id  INTEGER NOT NULL,
    context    TEXT    NOT NULL DEFAULT '',
    PRIMARY KEY (save_id, piece)
) WITHOUT ROWID;

-- "Which save is this row a piece of?" is asked by row.
CREATE INDEX IF NOT EXISTS idx_memory_piece_memory ON memory_piece (memory_id);
