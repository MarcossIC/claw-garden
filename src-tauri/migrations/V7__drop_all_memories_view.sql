-- Drops the `all_memories` UNION ALL view created by V3.
--
-- It was created for a single-query scan across the three memory kinds and no
-- query ever used it. The reason it stayed unused is structural, not an
-- oversight: the view flattens three tables onto one column list, so it cannot
-- expose `memory_type` (only the `memory` table has one) or any of the
-- per-kind columns its callers needed. `list_memories` reads each table
-- directly for exactly that reason, and retrieval scans the vector tables, not
-- this view.
--
-- Kept as a view it would have to be either maintained or explained on every
-- schema change, while offering a weaker interface than the typed queries that
-- already work.

DROP VIEW IF EXISTS all_memories;
