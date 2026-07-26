-- V4: Root Cause Analysis (doc 8, Fase 3). Nullable classification on
-- episodic_trajectory; NULL = "never classified" (valid). No backfill.
ALTER TABLE episodic_trajectory ADD COLUMN root_cause TEXT;
