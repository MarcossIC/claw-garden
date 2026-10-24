# import-decision-pass — Tareas

Strict TDD: en cada tarea con conducta nueva el test va primero y falla por la aserción o por no compilar. Una tarea, un commit convencional que compila y pasa la suite.

## 1. El asiento (ID2, sin cambio de conducta)

- [x] `Seat` con sus lecturas y escrituras, y `SqlSeat` sobre una transacción. `find`, `present_at`, `place`, `place_rows` y `write_links` pasan a ser genéricos sobre el asiento.
- [x] `record_projects` partido en `taken_projects` (pura) y sus `INSERT`.
- [x] `write_links` toma la cabecera de cada pieza de su línea.
- [x] `id` en el `ORDER BY` de `TWIN_SQL`. La guarda es `the_import_lookups_seek_an_index`, que sigue pasando.
- [x] El oráculo de la exploración pasa sin editar ninguna prueba.

## 2. El planificador (ID1, ID2, ID3, ID4)

- [x] Tests primero (`import.rs`):
  - `the_planner_decides_as_the_sql_pass`, la tabla de la spec, que no compila sin el planificador;
  - `a_dry_run_leaves_the_connection_to_other_calls`, que falla hoy por la espera de la sonda;
  - `a_stopped_planner_decides_nothing`;
  - `a_dry_run_after_a_purge_reports_the_ids_of_the_import`, una guarda que hoy pasa;
  - `a_dry_run_reports_without_writing`, ampliado a `project_remote`, `memory_fts` y `sqlite_sequence`;
  - `the_import_lookups_seek_an_index`, con las consultas de carga.
- [x] `Planned` y su carga (`ORIGIN_ROWS_SQL`, `TEXT_ROWS_SQL`, `KEY_ROWS_SQL`, `PROJECT_ROOT_SQL`, `LINKED_SQL` y el próximo id), en llamadas de `LOAD_BATCH` búsquedas.
- [x] `write` decide con el planificador en `spawn_blocking`, con el indicador de parada. La pasada SQL queda sólo en la prueba diferencial.
- [x] El párrafo de cabecera de `import.rs`.

## 3. Docs

- [x] `.claude/CLAUDE.md` (la pasada en `clawg import`) y la fila 82 de `openspec/README.md`. El delta de `specs/portability` y el orden de ejecución van con el archivo.

## 4. Verificación

- [x] Suite y clippy de `.claude/CLAUDE.md` en Linux.
- [x] El banco de 81 y la espera máxima de la sonda, antes y después, en `verify.md`.
- [x] Suite y clippy en Windows (`target-test`).
- [x] El CI en verde en Linux y Windows.
- [x] **[perfil dev, con autorización]** Una importación real con sondas (`/health`, listado y búsqueda): la fase de decisión no agota ningún plazo.
- [x] `verify.md`.

## 5. Archive

- [x] `specs/portability` con ID1 a ID3, e ID4 en la tabla de IL1; `git mv` a `changes/archive/2026-10-24-import-decision-pass`; `state.yaml` archivado; la fila 82 y el orden de ejecución del `README.md` de openspec; subir el minor. También se corrigió el comentario de `the_import_lookups_seek_an_index`, que import-linear dejó anotado ([verify](verify.md#al-archivar-2026-10-24)).
