# import-linear — Tareas

Strict TDD: en cada tarea que agrega conducta nueva, el test va primero y falla por su aserción. Cada tarea es un commit convencional.

## 1. Docs del cambio

- [x] Propuesta, spec, tareas y `state.yaml`.
- Commit: `docs(openspec): write the proposal, spec and tasks of import-linear` (`8c1f747`)

## 2. Las búsquedas de la importación, por índice (R1–R4)

- [x] Tests primero: `the_import_lookups_seek_an_index` (`import.rs`) y `a_store_from_before_the_unfiltered_dedup_index_keeps_its_rows` (`db.rs`). Los dos fallaron por su aserción: `PRESENT_SQL` daba `SCAN memory`, y el índice de un store en V24 conservaba su `WHERE` ([verify](verify.md#red-antes-del-cambio)).
- [x] Las dos consultas de `memory_piece` de `write_links` pasaron a ser constantes (`LINKED_SQL`, `NEXT_SAVE_SQL`), y la prueba de planes las explica.
- [x] `V25__dedup_index_all_rows.sql`, su fila en `published_migration_checksums_are_pinned` (que también es el toque a `migrations.rs` que fuerza el recompilado) y las tres cuentas de migraciones de `db.rs` en 25.
- [x] Toda la suite y clippy en verde, con `reinforce_lookup_uses_dedup_index` y `the_scope_lookup_uses_the_topic_index` sin cambios.
- Commit: `perf(core): look up imported rows by index` (`1f0ab8c`)

## 3. Banco de escala (R5)

- [x] `twice_the_notes_import_in_at_most_three_times_the_time` (`import.rs`, `#[ignore]`). Con el índice parcial de antes falla (3,80 veces); con V25 pasa (de 1,96 a 2,38 veces en tres corridas). El informe de cada tamaño tiene el mismo hash en las dos versiones ([verify](verify.md#banco-de-escala)).
- Commit: `test(core): measure how an import scales with its rows` (`f220182`)

## 4. `prepare_cached`, sólo si la medición lo pide

- [x] La medición lo pidió: con el índice puesto, preparar `PRESENT_SQL` y `TWIN_SQL` en cada fila costaba ~16 % del banco. Las tres llamadas (`place` y `present_at`) pasaron a `prepare_cached`, como un refactor con la suite en verde.
- Commit: `perf(core): prepare the import's per-row lookups once` (`f967979`)

## 5. Verificación

- [x] Toda la suite y clippy en Windows sobre el último commit, y el banco antes y después ([verify](verify.md)).
- [x] Con autorización explícita: una importación real en el perfil de desarrollo, con compilación de depuración. Tardó 1 218 s contra 3 078 s, y el dry run 70 s contra 910 s, con las mismas filas ([verify](verify.md#importación-real-en-el-perfil-de-desarrollo-2026-10-24)).
- [x] El CI en verde en Linux y Windows: el del pull request #87 sobre `42b1aec` y sobre `076eeb7` ([verify](verify.md#ci-2026-10-24)).

## 6. Archive

- [x] `specs/storage` (V25 y su vuelta atrás como IL2, R3 como IL3, checksums hasta V25, `idx_memory_dedup` ya no parcial en las secciones de V18 y V20) y `specs/portability` (R1 y R5 como IL1 e IL5); `git mv` a `changes/archive/2026-10-24-import-linear`; `state.yaml` archivado; fila 81 y orden de ejecución del `README.md` de openspec; subir el minor. También se realinearon las citas a `import.rs`, `db.rs` y `migrations.rs` ([verify](verify.md#al-archivar-2026-10-24)).
