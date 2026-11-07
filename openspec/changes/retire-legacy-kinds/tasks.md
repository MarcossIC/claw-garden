# retire-legacy-kinds — Tareas

Strict TDD: los tests de cada tarea van primero y fallan, por la aserción o porque no compilan. Cada tarea es un commit convencional que compila y pasa los tests de los archivos que cambia; la suite entera y los dos clippy corren una vez, sobre el último commit.

La tarea 2 es un solo commit grande: V32 borra las tablas, así que todo lo que las leía tiene que irse en el mismo commit, y sacar `MemoryKind` del núcleo cambia las firmas que usa el daemon. Partirlo dejaría commits que no compilan o que esconden filas.

## 1. Artefactos

- [x] `proposal.md`, `spec.md`, `design.md`, `tasks.md` y `state.yaml`.
- Commit: `docs(openspec): propose retire-legacy-kinds`

## 2. La migración y el núcleo (R1, R2, R3, R4, R5, R8, R9, R10)

- [x] Tests en rojo: `trajectories_and_skills_move_to_memory_and_their_tables_go`, `a_project_deleted_before_the_move_comes_back_with_its_trajectories_and_skills`, `a_merge_from_before_the_move_takes_its_trajectories_and_skills_back`, `a_trajectory_and_a_skill_deleted_before_the_move_are_restored_and_purged` y `a_store_moved_from_trajectories_and_skills_breaks_no_invariant` (`migrations.rs`); `no_production_code_names_the_retired_tables` (`conventions.rs`); `a_search_records_how_long_each_arm_took` (`search.rs`); `an_export_of_a_store_before_the_move_counts_the_rows_it_leaves` (`export.rs`); `a_retired_kind_is_refused_with_where_its_rows_went` y `the_answers_keep_what_a_bridge_of_the_previous_version_requires` (`tests/api.rs`); `parse_kind_takes_memory_alone` y `a_retired_kind_says_where_its_rows_went` (`api.rs`).
- [x] `V32__retire_legacy_kinds.sql`, `migrations.rs` tocado y su checksum fijado.
- [x] Núcleo: búsqueda, puntaje, mantenimiento, tier, proyectos (registro, unión, borrado, purga), deshechura de importaciones, `inspect`, espacio de trabajo, exportación, `record.rs`, `store_insert.rs` y `admin.rs`/`pieces.rs` sin `kind`; `eval_api.rs` sin `EvalKind` ni los dos pools.
- [x] Daemon: `parse_kind` con la explicación, el envoltorio que agrega `kind` a las respuestas (las de `synthesis` también), los dos conteos de `GET /v1/stats` en 0 y los de la unión de una sugerencia respondida.
- [x] CLI y bridge, lo que la compilación pide: `Counts`, `MergeReport`, `IdRows`, `Kept`, `LiveRow` y `StatsTotals` sin trayectorias ni habilidades, en `projects.rs`, `secrets.rs`, `memories.rs`, `export.rs` y `doctor/checks.rs`.
- [x] `rag-eval` sin los dos pools en `K_POOL` ni en `k_pool`.
- Commit: `feat(core): move trajectory and skill rows into memory and drop their tables`

## 3. CLI y MCP (R6, R7)

- [x] Tests en rojo: `a_retired_kind_is_a_usage_error_that_says_where_its_rows_went` (`cli.rs`), `a_retired_kind_is_answered_with_where_its_rows_went` (`tools.rs`), `a_retired_kind_is_answered_without_calling_the_daemon` (`server.rs`).
- [x] `--kind` oculto con su `value_parser`; `KindArg` se va.
- [x] `kind` fuera del esquema de las cinco herramientas, leído y comprobado en el bridge; descripciones sin tablas ni ids por tabla; los dos contratos bendecidos con `CLAWG_BLESS=1`.
- Commit: `feat(cli): say where the rows of a retired kind went`

## 4. `doctor` sobre el binario (R8)

- [x] `doctor_passes_over_a_store_whose_trajectories_and_skills_moved` (`binary.rs`): una copia congelada en V31, sembrada por SQL, abierta con el binario nuevo.
- Commit: `test(cli): run doctor over a store whose trajectories and skills moved`

## 5. Docs

- [x] `docs/reference/` (`commands.md`, `daemon.md`, `data-profiles.md`, `testing.md` donde nombre los pools) y `README.md`.
- Commit: `docs: describe the retirement of the trajectory and skill tables`

## 6. Verificación

- [x] `cargo test --workspace` y `cargo clippy --workspace --all-targets -- -D warnings` sobre el último commit de código, en Linux.
- [ ] El CI del PR, que corre también Windows (`cfg(windows)`).
- [x] `verify.md` con lo que queda pendiente en local: la migración sobre una copia de `prod` y otra de `dev`, `search.duration` antes y después sobre una copia, y `rag-eval --diff` con el modelo real y el golden de `docs/eval/`.
- Commit: `docs(openspec): record retire-legacy-kinds applied`

## 7. Archivo

Fuera de esta tarea: consolidar las specs, mover el cambio al archivo y subir el minor.
