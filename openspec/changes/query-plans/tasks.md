# query-plans — Tareas

Strict TDD: en cada tarea con conducta nueva el test va primero y falla por la aserción. Una tarea, un commit convencional.

## 1. Los índices de las sentencias de sesión (R1–R4; D1, D1b, D2, D4)

- [x] Tests primero: `the_session_statements_read_the_newest_rows_by_index_without_sorting` y `the_session_statements_order_rows_of_one_stamp_by_id` (`prompt_builder.rs`); `a_store_from_before_the_session_indexes_keeps_its_rows_and_gains_them` (`db.rs`). Los dos de plan y migración fallan por la aserción; el de orden pasa antes y después.
- [x] `V18__session_indexes.sql` (con `idx_memory_dedup` reconstruido, D1b, y la aserción sin `TEMP B-TREE` en `reinforce_lookup_uses_dedup_index`), su fila en `published_migration_checksums_are_pinned`, el conteo de migraciones en los tests que lo fijan, `project_id IS ?1` en `LAST_SESSION_SQL` y `prepare_cached` en las dos sentencias.
- Commit: `perf(core): read the last session and recent rows by index`

## 2. Docs del cambio

- [x] Propuesta, spec, diseño, tareas, verify y `state.yaml`.
- Commit: `docs: plan the session statement indexes`

## 3. Verificación

- [x] Suite y clippy en Linux sobre el último commit ([verify](verify.md)).
- [x] Estadísticas evaluadas con las pruebas de plan sobre un store con `ANALYZE` (D5).
- [x] Tiempo antes y después con filas generadas.
- [ ] El CI en verde en Linux y Windows.

## 4. Archive

- [x] `specs/storage` (V18 y su vuelta atrás, checksums hasta V18, R3 y R4) y `specs/retrieval` (R1, R2), citas de `prompt_builder.rs` realineadas en las specs y el `README.md`; `git mv` a `changes/archive/2026-10-21-query-plans`; `state.yaml` archivado; fila 56, el backlog que resolvió y el orden de ejecución del `README.md` de openspec; subir el minor.
