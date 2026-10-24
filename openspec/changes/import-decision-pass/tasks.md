# import-decision-pass — Tareas

Strict TDD: en cada tarea con conducta nueva el test va primero y falla por la aserción o por no compilar. Una tarea, un commit convencional que compila y pasa la suite.

## 1. El asiento (ID2, sin cambio de conducta)

- [ ] `Seat` con sus lecturas y escrituras, y `SqlSeat` sobre una transacción. `find`, `present_at`, `place`, `place_rows` y `write_links` pasan a ser genéricos sobre el asiento.
- [ ] `record_projects` partido en `taken_projects` (pura) y sus `INSERT`.
- [ ] `write_links` toma la cabecera de cada pieza de su línea.
- [ ] `id` en el `ORDER BY` de `TWIN_SQL`. La guarda es `the_import_lookups_seek_an_index`, que sigue pasando.
- [ ] El oráculo de la exploración pasa sin editar ninguna prueba.

## 2. El planificador (ID1, ID2, ID3, ID4)

- [ ] Tests primero (`import.rs`):
  - `the_planner_decides_as_the_sql_pass`, la tabla de la spec, que no compila sin el planificador;
  - `a_dry_run_leaves_the_connection_to_other_calls`, que falla hoy por la espera de la sonda;
  - `a_stopped_planner_decides_nothing`;
  - `a_dry_run_after_a_purge_reports_the_ids_of_the_import`, una guarda que hoy pasa;
  - `a_dry_run_reports_without_writing`, ampliado a `project_remote`, `memory_fts` y `sqlite_sequence`;
  - `the_import_lookups_seek_an_index`, con las consultas de carga.
- [ ] `Planned` y su carga (`ORIGIN_ROWS_SQL`, `TEXT_ROWS_SQL`, `KEY_ROWS_SQL`, `PROJECT_ROOT_SQL`, `LINKED_SQL` y el próximo id), en llamadas de `LOAD_BATCH` búsquedas.
- [ ] `write` decide con el planificador en `spawn_blocking`, con el indicador de parada. La pasada SQL queda sólo en la prueba diferencial.
- [ ] El párrafo de cabecera de `import.rs`.

## 3. Docs

- [ ] Delta de `specs/portability` (ID1 a ID4), `.claude/CLAUDE.md` si nombra la pasada, la fila 82 y el orden de ejecución de `openspec/README.md`.

## 4. Verificación

- [ ] Suite y clippy de `.claude/CLAUDE.md` en Linux.
- [ ] El banco de 81 y la espera máxima de la sonda, antes y después, en `verify.md`.
- [ ] Suite y clippy en Windows (`target-test`).
- [ ] El CI en verde en Linux y Windows.
- [ ] **[perfil dev, con autorización]** Una importación real con sondas (`/health`, listado y búsqueda): la fase de decisión no agota ningún plazo.
- [ ] `verify.md`.

## 5. Archive

- [ ] `specs/portability` con ID1 a ID4; `git mv` a `changes/archive/AAAA-MM-DD-import-decision-pass`; `state.yaml` archivado; la fila 82 y el orden de ejecución del `README.md` de openspec; subir el minor.
