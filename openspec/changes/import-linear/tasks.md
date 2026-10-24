# import-linear — Tareas

Strict TDD: en cada tarea que agrega conducta nueva, el test va primero y falla por su aserción. Cada tarea es un commit convencional.

## 1. Docs del cambio

- [x] Propuesta, spec, tareas y `state.yaml`.
- Commit: `docs(openspec): write the proposal, spec and tasks of import-linear`

## 2. Las búsquedas de la importación, por índice (R1–R4)

- [ ] Tests primero: `the_import_lookups_seek_an_index` (`import.rs`) y `a_store_from_before_the_unfiltered_dedup_index_keeps_its_rows` (`db.rs`). Los dos fallan por su aserción: `PRESENT_SQL` da `SCAN memory`, y el índice de un store en V24 conserva su `WHERE`.
- [ ] Las dos consultas de `memory_piece` de `write_links` pasan a constantes, y la prueba de planes las explica.
- [ ] `V25__dedup_index_all_rows.sql`, su fila en `published_migration_checksums_are_pinned`, el toque a `migrations.rs` que fuerza el recompilado y las tres cuentas de migraciones de `db.rs` en 25.
- [ ] Toda la suite y clippy en verde, `reinforce_lookup_uses_dedup_index` y `the_scope_lookup_uses_the_topic_index` sin cambios entre ellas.
- Commit: `perf(core): look up imported rows by index`

## 3. Banco de escala (R5)

- [ ] `twice_the_notes_import_in_at_most_three_times_the_time` (`import.rs`, `#[ignore]`): se corre antes del cambio para confirmar que falla y después para confirmar que pasa, en la misma compilación y con el informe de cada tamaño igual en las dos corridas.
- Commit: `test(core): measure how an import scales with its rows`

## 4. `prepare_cached`, sólo si la medición lo pide

- [ ] Con el índice puesto, se mira si preparar `PRESENT_SQL` y `TWIN_SQL` en cada fila pesa en el banco. Si pesa, se pasan a `prepare_cached` (`import.rs:481`, `:572`, `:579`) en un commit propio; si no, la tarea se cierra sin cambios y se anota en verify.

## 5. Verificación

- [ ] Toda la suite y clippy en Windows sobre el último commit, y el banco antes y después ([verify](verify.md)).
- [ ] Con autorización explícita: una importación real en el perfil de desarrollo, con compilación de depuración, para comparar con los 3 078 s.
- [ ] El CI en verde en Linux y Windows.

## 6. Archive

- [ ] `specs/storage` (V25 y su vuelta atrás, checksums hasta V25, `idx_memory_dedup` ya no parcial en `:375` y `:397`) y `specs/portability` (R1, R5); `git mv` a `changes/archive/AAAA-MM-DD-import-linear`; `state.yaml` archivado; fila 81 y orden de ejecución del `README.md` de openspec; subir el minor.
