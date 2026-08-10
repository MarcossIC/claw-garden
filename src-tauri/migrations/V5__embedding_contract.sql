-- Single-row table recording which embedder produced the vectors currently
-- stored in memory_vectors. A store opened with a different model_id or dim
-- than this row is comparing incompatible vector spaces, which is silent
-- corruption rather than an error unless something checks it explicitly.
CREATE TABLE IF NOT EXISTS embedding_contract (
    id              INTEGER PRIMARY KEY CHECK (id = 1),
    model_id        TEXT    NOT NULL,
    dim             INTEGER NOT NULL,
    metric          TEXT    NOT NULL,
    normalized      INTEGER NOT NULL,
    query_prefix    TEXT    NOT NULL DEFAULT '',
    passage_prefix  TEXT    NOT NULL DEFAULT '',
    created_at      INTEGER NOT NULL
);
