-- content_hash: hex SHA-256 of the chunk's normalized text (trim + NFC),
-- computed by the `content_hash` UDF registered before migrations run
-- (see db.rs::register_udfs). Backfilled eagerly so every pre-existing row
-- is dedup-eligible immediately, not lazily on next write.
ALTER TABLE memory ADD COLUMN content_hash TEXT;

UPDATE memory SET content_hash = content_hash(content) WHERE content IS NOT NULL;

-- Not UNIQUE: rows written before this column existed can already hold repeated
-- content, so a unique constraint would fail the moment the backfill lands on a
-- live database. Uniqueness is enforced by the ingest worker, which drains its
-- queue as the single sequential consumer. COALESCE is required because
-- project_id is nullable and SQLite treats NULL as distinct from NULL.
CREATE INDEX IF NOT EXISTS idx_memory_dedup
    ON memory (agent_id, COALESCE(project_id, ''), content_hash)
    WHERE deleted_at IS NULL AND content_hash IS NOT NULL;
