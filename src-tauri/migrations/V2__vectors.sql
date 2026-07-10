-- V2: vec0 virtual tables for 384-dimensional FLOAT32 embeddings.
-- Each virtual table mirrors one regular table — rowid matches the parent table id.
-- Splitting from V1 ensures a vec0 registration failure produces a clean
-- "no such module: vec0" error instead of corrupting the V1 schema.

CREATE VIRTUAL TABLE IF NOT EXISTS memory_vectors
    USING vec0(embedding float[384]);

CREATE VIRTUAL TABLE IF NOT EXISTS episodic_vectors
    USING vec0(embedding float[384]);

CREATE VIRTUAL TABLE IF NOT EXISTS procedural_vectors
    USING vec0(embedding float[384]);
