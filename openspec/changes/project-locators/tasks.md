# project-locators — Tareas

Strict TDD: en cada tarea, el test va primero y falla, por la aserción o porque no compila. Cada tarea es un commit convencional que compila y pasa la suite (`cargo test --workspace --target-dir target-test` en Windows) y clippy (`cargo clippy --workspace --all-targets --target-dir target-test -- -A clippy::all -D warnings`). Antes de empezar una entrega: `cargo clean --workspace --target-dir target-test`. Las tareas de las entregas 2b a 4 se escriben cuando les toque. Ninguna entrega sube la versión: el cambio sale entero en 0.64.0.

## Entrega 1. El registro (V27)

Requisitos en la [spec](spec.md) (PL1 a PL10) y decisiones en el [diseño](design.md) (D1 a D14).

### 1. V27: `project` como referencia, `project_path` para las carpetas, y la adopción (PL1, PL2, PL8; D1, D6, D7)

- [ ] RED:
  - `a_database_from_the_previous_version_opens_with_a_project_for_every_id_in_use` (`db.rs`, sembrado con `seed_on_disk(dir, Some(26))` y el escenario de PL1);
  - `a_row_saved_under_an_unknown_project_adopts_it`, `a_row_moved_to_an_unknown_project_adopts_it` y `the_name_of_an_adopted_id_is_the_one_rust_gives` (`crates/clawg-core/src/memory/registry.rs`);
  - `a_project_without_a_folder_is_a_target` (`relocate.rs`);
  - la fila 27 en `published_migration_checksums_are_pinned` (`migrations.rs`).
- [ ] GREEN:
  - `crates/clawg-core/migrations/V27__project_registry.sql`: el renombre, las columnas de `project_path`, las tablas `project` y `project_suggestion`, los índices, el relleno y los dos disparadores;
  - tocar `migrations.rs` en el mismo commit, para que se recompile;
  - `registry.rs` con `project_name`;
  - cada lector y escritor de carpetas pasa a `project_path`: `project.rs`, `relocate.rs` (salvo `check_target_in`, que sigue en `project`), `project_docs/mod.rs`, `import.rs`, `undo.rs`;
  - los que abren stores sin migrar eligen la tabla según el esquema: `inspect.rs`, `export.rs` (`read` y `project_id_of`);
  - los tests que insertan o cuentan carpetas pasan a `project_path`. Los de migraciones anteriores las leen de `project_path` después de abrir, sin cambiar lo que comprueban.
- Commit: `feat(core): make project the reference of every project and keep folders in project_path`

### 2. Las huellas de un checkout (PL3; D3)

- [ ] RED:
  - en `crates/clawg-core/tests/project_root.rs`, con repositorios reales creados con `git` en directorios temporales: `the_first_commit_is_the_root_of_the_first_parent_chain` (un merge de una historia ajena no lo cambia), `a_shallow_or_empty_repository_has_no_first_commit_to_record` (un `clone --depth 1` desde `file://`, y un `git init` sin commits) y `the_stamp_of_a_checkout_changes_when_it_is_cloned_again`;
  - `a_command_that_does_not_end_in_time_is_stopped` (`identity.rs`, con un `ping` o un `sleep`, como el test de `serve.rs`).
- [ ] GREEN:
  - en `identity.rs`: `git_stamp`, `root_commit` (con el resultado de tres casos de PL3), la espera con plazo, `CREATE_NO_WINDOW` en Windows y el entorno sin `GIT_*`;
  - el trait `Ground` y su versión real, `Disk`.
- Commit: `feat(core): read the first commit and the git directory of a checkout`

### 3. Una carpeta sirve a una raíz sólo si sus huellas no la contradicen (PL3, PL4, PL7; D4, D9, D13)

- [ ] RED:
  - la pura `a_checkout_settles_on_the_folder_its_fingerprints_do_not_contradict`;
  - sobre `MemoryStore::new_for_test()` con un `Ground` falso que cuenta las lecturas: `a_recorded_checkout_does_not_read_its_first_commit_again`, `a_repository_without_origin_in_the_path_of_another_gets_a_project_of_its_own`, `two_repositories_without_origin_in_one_path_are_two_projects`, `a_remote_decides_before_the_first_commit`, `a_clone_folder_records_its_own_remote` y `a_caller_that_derives_its_id_is_refused_a_repository_without_origin_in_its_path` (`project.rs`);
  - `a_repository_without_origin_replaced_in_its_path_registers_into_a_project_of_its_own` (`crates/clawg-cli/tests/api.rs`, con repositorios reales);
  - los tests de R4 y de R14 a R20 de `project.rs` pasan a usar el `Ground` falso y siguen pasando sin cambiar lo que comprueban.
- [ ] GREEN:
  - `fit` y `settle` sobre huellas, y `path_rows_in` con las columnas nuevas;
  - la pasada que pide lo que falta, y `Registration::checkout`;
  - `write_in` graba `project_id`, `remote`, `root_commit` y `git_stamp`, y crea el proyecto `root` que falte;
  - `register` lee las huellas con `Disk`;
  - `project_id_in`, `plan_ids_in` y `project_id_of` con `Ground`, y sus llamadores en `memories.rs` y `main.rs`.
- Commit: `fix(core): tell two repositories in one path apart by their first commit`

### 4. Mudanzas y sugerencias (PL5, PL6, PL7; D5, D10)

- [ ] RED:
  - `a_checkout_moved_without_origin_keeps_its_project`, `a_copy_of_a_checkout_is_a_project_of_its_own_with_a_suggestion`, `a_checkout_two_projects_could_be_is_a_project_of_its_own`, `a_renamed_remote_in_its_path_is_another_project_with_a_suggestion`, `a_suggestion_is_made_once_per_pair`, `a_moved_checkout_is_read_and_planned_alike_without_writing` y `a_caller_that_derives_its_id_never_follows_a_move` (`project.rs`);
  - `a_folder_moved_without_origin_registers_into_its_project` (`tests/api.rs`, con repositorios reales).
- [ ] GREEN:
  - `moved` (pura);
  - en la pasada, leer los checkouts grabados con ese primer commit;
  - la carpeta nueva vinculada, las sugerencias (`INSERT OR IGNORE`) y sus líneas de log.
- Commit: `feat(core): keep the project of a folder moved without origin`

### 5. El import escribe sus carpetas como un registro (PL7; D12)

- [ ] RED: `an_import_records_its_folders_as_a_registration_would` (`import.rs`). Comprueba tres cosas:
  - una raíz del archivo que es la mudanza de un checkout de aquí manda sus filas al proyecto del plan, y su carpeta lo dice;
  - una raíz con remoto graba ese remoto;
  - el proyecto que no existía queda en `project` con `born = 'import'`.
- [ ] GREEN:
  - `roots_here` lee las huellas en un hilo bloqueante;
  - `record_projects` inserta las columnas nuevas y el proyecto;
  - el import no crea sugerencias.
- Commit: `fix(core): record an imported folder as a registration would`

### 6. Listar y renombrar en el daemon (PL9; D8, D11)

- [ ] RED:
  - `renaming_keeps_the_id_and_marks_the_project_reviewed`, `a_name_another_project_holds_is_refused` y `a_name_is_trimmed_and_bounded` (`registry.rs`);
  - `the_projects_are_listed_with_their_folders_and_suggestions` (con un proyecto sin carpeta y una carpeta que ya no está), `renaming_a_project_keeps_what_was_saved_under_it` y `renaming_an_unknown_project_is_404` (`tests/api.rs`).
- [ ] GREEN:
  - `registry::list` y `registry::rename`;
  - `GET /v1/projects` y `PATCH /v1/projects/{id}` detrás del token, con la clave y la presencia de cada carpeta leídas en un hilo bloqueante.
- Commit: `feat(daemon): list and rename projects`

### 7. `clawg projects list` y `clawg projects rename` (PL9)

- [ ] RED: `projects_list_and_rename_parse` (`cli.rs`); `a_listing_marks_what_was_created_alone_and_the_suggestions` (`projects.rs`, en español y en inglés).
- [ ] GREEN: `ProjectsAction::List` y `ProjectsAction::Rename`, los métodos del cliente y la impresión, con una sentencia por `match` sobre `Lang`.
- Commit: `feat(cli): add projects list and rename`

### 8. Las specs y la documentación

- [ ] `specs/project`:
  - PL1 a PL10;
  - R10, R16, R18 y R20 al día;
  - las definiciones de proyecto y carpeta;
  - el «Origen».
- [ ] `specs/storage`: V27 (el renombre, el relleno, los disparadores) y su vuelta atrás.
- [ ] `specs/doctor`: la regla `projects` lee las carpetas de `project_path`.
- [ ] `.claude/CLAUDE.md`:
  - el párrafo del proyecto en «Data profiles»: proyecto y carpeta, el primer commit, las mudanzas y las sugerencias;
  - `clawg projects list|rename` en «Commands»;
  - las dos rutas nuevas en la API.
- [ ] El contrato del modelo no cambia: `the_agent_contract_matches_its_file` pasa sin bendecir.
- Commit: `docs: record projects as the reference and folders as one way to reach them`

### 9. La versión

- [ ] Si 0.63.0 ya se publicó: `0.63.0` → `0.64.0` (la entrega se publica sola y trae esquema y comandos nuevos), y `cargo update --workspace`. Si no se publicó, la entrega sale con la 0 de 89 en 0.63.0 y la versión no sube (como pasó al [archivar 89](../archive/2026-10-25-project-registry/tasks.md)).
- Commit: `chore: bump version to 0.64.0`

### 10. Verificación

- [ ] La suite y clippy de `.claude/CLAUDE.md`, en `target-test` en Windows.
- [ ] El binario en un home temporal, con repositorios reales:
  - una carpeta sin `origin` registrada, movida y registrada de nuevo conserva su proyecto;
  - otra en su camino viejo recibe uno propio;
  - `projects list` muestra la sugerencia de una copia y un proyecto cuya carpeta se borró;
  - `projects rename` conserva lo guardado.
- [ ] El CI en Linux y Windows, en el pull request final del cambio.
- [ ] `verify.md`, sección «Entrega 1».
- [ ] No queda otro directorio de build que `target` y `target-test`.

## Entrega 2a. Unir proyectos (V28)

Requisitos en la [spec](spec.md#entrega-2a-unir-proyectos) (PL11 a PL20) y decisiones en el [diseño](design.md#entrega-2a-unir-proyectos) (D15 a D25). Antes de la tarea 11: `cargo clean --workspace --target-dir target-test`.

### 11. V28 y `canonical` (PL11, PL12; D15, D16)

- [ ] RED:
  - `a_database_from_the_previous_version_opens_with_every_project_live` (`db.rs`, sembrado con `seed_on_disk(dir, Some(27))` y el escenario de PL11);
  - `canonical_follows_a_merge_and_leaves_any_other_id_as_it_is` y `a_cycle_or_a_chain_too_long_answers_the_id_itself` (`crates/clawg-core/src/memory/merge.rs`, con lápidas escritas a mano);
  - la fila 28 en `published_migration_checksums_are_pinned` (`migrations.rs`).
- [ ] GREEN:
  - `crates/clawg-core/migrations/V28__project_merge.sql`: las dos columnas, el índice parcial, `project_merge` y `project_merge_change`;
  - tocar `migrations.rs` en el mismo commit;
  - `merge.rs` con `canonical_in(c, id)` y `MAX_MERGE_HOPS`.
- Commit: `feat(core): add the tombstone of a merged project`

### 12. Unir X en Z en el store (PL13; D17 a D21)

- [ ] RED (`merge.rs`, sobre `MemoryStore::new_for_test()`):
  - `merging_moves_every_row_and_locator_into_the_target`;
  - `a_merge_keeps_the_newest_save_of_a_key_and_the_oldest_of_a_text`;
  - `a_merge_moves_the_part_of_a_split_unit_it_holds`;
  - `merging_a_target_repoints_its_tombstones`;
  - `a_dry_run_merge_changes_nothing`;
  - `a_merge_copies_a_workspace_only_where_the_target_has_none`;
  - `a_merge_is_refused_while_a_save_of_its_projects_is_pending`, y `a_merge_of_a_tombstone_into_its_target_repairs_it`.
- [ ] GREEN:
  - `Span::{Keep, Carry}` en `decide_merge` (`relocate.rs`): el registro pasa `Keep`;
  - `merge_core(store, workspaces, backups, source, target, dry_run)`: la pasada en seco, la espera de `SAVES_WAIT`, el snapshot, la transacción que vuelve a decidir y el diario;
  - `MergeError` con `thiserror`;
  - `Reason::PreMerge` (`backup.rs`);
  - `adopt_legacy` devuelve `Adopted { copied, kept_beside }`, y el registro lo ignora.
- Commit: `feat(core): merge one project into another`

### 13. Deshacer una unión en el store (PL14; D20)

- [ ] RED (`merge.rs`): `undoing_a_merge_leaves_the_store_as_it_was`, `undoing_a_merge_keeps_what_was_saved_after_it`, `a_merge_whose_target_was_merged_since_is_undone_after_that_one` y `a_workspace_edited_since_the_merge_is_kept`.
- [ ] GREEN:
  - el cuerpo de `undo_move_core` pasa a `undo_move_in(c, move_id, write)`, que la unión llama dentro de su transacción. `undo_move_core` sigue igual por fuera;
  - `undo_merge_core`, con `pre-undo` y el borrado de copias por la parte del usuario.
- Commit: `feat(core): undo a merge of projects`

### 14. Los escritores siguen la lápida (PL12; D17)

- [ ] RED:
  - `a_save_retried_after_a_merge_lands_in_its_target` (`worker.rs`);
  - `a_move_to_a_merged_project_lands_in_its_target` y `undoing_a_move_whose_origin_was_merged_returns_to_its_target` (`relocate.rs`);
  - `a_root_whose_project_was_merged_registers_into_its_target`, también leyendo y planeando sin escribir (`project.rs`);
  - `an_import_planned_before_a_merge_writes_into_its_target` (`import.rs`).
- [ ] GREEN:
  - `canonical_in` al empezar `process_tracked` (`worker.rs:676`);
  - `check_target_in` devuelve el destino vivo, y el `--undo` de un movimiento resuelve el origen de cada fila;
  - `plan_in` y `plan_ids_in` resuelven el proyecto que reciben, después de `project_of` y de una mudanza;
  - el import resuelve el proyecto de cada fila en la transacción que la escribe.
- Commit: `fix(core): write under the target of a merged project`

### 15. Las entradas del daemon siguen la lápida (PL12, PL19; D23, D24)

- [ ] RED (`tests/api.rs`, con una unión hecha en el store del `Harness`):
  - `every_route_says_which_project_it_takes`, que lee `router()` de `api.rs`;
  - `every_route_that_takes_a_project_follows_a_merge`;
  - `a_bridge_opened_before_a_merge_reads_and_writes_its_target`;
  - `a_move_from_a_merged_project_is_refused` y `the_move_of_a_merge_is_undone_only_with_its_merge`.
- [ ] GREEN:
  - un ayudante del daemon que resuelve un `Option<String>` de proyecto con `canonical_in`, en `save_memory`, `search`, `context`, `register_session`, `synthesis`, `disputes`, `list_memories` y `stats`;
  - 409 `project_merged` para `rename` y `from_project`, y 409 `part_of_merge` para el `undo` de un movimiento.
- Commit: `fix(daemon): follow a merged project at every entry that takes one`

### 16. La lista y `keep-apart` en el store (PL15, PL16; D22)

- [ ] RED:
  - `a_merged_project_is_listed_under_its_target_and_frees_its_name` y `keeping_two_projects_apart_answers_their_suggestion_for_good` (`registry.rs`);
  - `a_pair_kept_apart_is_not_suggested_again` (`project.rs`).
- [ ] GREEN:
  - `list_projects` sin lápidas, con `merged` y las sugerencias por `canonical`;
  - `rename_project` con `RenameError::Merged`, y el nombre de una lápida libre;
  - `keep_apart`.
- Commit: `feat(core): list a merged project under its target and keep two projects apart`

### 17. Unir y separar en el daemon (PL13, PL14, PL16)

- [ ] RED (`tests/api.rs`): `merging_through_the_daemon_leaves_everything_under_the_target`, `a_merge_waits_for_the_saves_of_its_projects` y `the_listing_after_a_merge_shows_the_target_alone`. Las dos rutas nuevas entran a la tabla de `every_route_says_which_project_it_takes`.
- [ ] GREEN: `POST /v1/projects/merge` y `POST /v1/projects/keep-apart` detrás del token, con el mapeo de `MergeError` y de los rechazos de PL13, PL14 y PL16.
- Commit: `feat(daemon): merge projects and keep them apart`

### 18. `clawg projects merge` y `clawg projects keep-apart` (PL17)

- [ ] RED: `projects_merge_and_keep_apart_parse` (`cli.rs`); `a_merge_report_names_what_moved_and_the_way_back` y `a_listing_names_what_was_merged_into_a_project` (`projects.rs`, en español y en inglés).
- [ ] GREEN: `ProjectsAction::Merge` y `ProjectsAction::KeepApart`, los métodos del cliente, `render_merge` y la línea de `render_list`, con una sentencia por `match` sobre `Lang`.
- Commit: `feat(cli): add projects merge and keep-apart`

### 19. Los hallazgos de `doctor` (PL18; D25)

- [ ] RED: `the_duplicates_are_named_by_name_and_by_first_commit`, `a_pair_kept_apart_is_no_duplicate`, `an_id_without_a_project_is_named` y `rows_under_a_merged_project_and_a_broken_chain_are_named` (`checks.rs`); la lectura de los hechos nuevos en un store en V26, en V27 y en V28 (`inspect.rs`).
- [ ] GREEN: los hechos en `inspect.rs`, los tres chequeos en `checks.rs` con sus mensajes en los dos idiomas, su llamada en `doctor/mod.rs` y su lugar en `every_finding`.
- Commit: `feat(doctor): name duplicate projects, ids without a project and broken merges`

### 20. Las specs y la documentación

- [ ] `specs/project`: PL11 a PL20, y PL9 con lo que cambia PL15.
- [ ] `specs/storage`: V28, su vuelta atrás y el motivo `pre-merge`.
- [ ] `specs/doctor`: los tres chequeos.
- [ ] `.claude/CLAUDE.md`:
  - el párrafo del proyecto en «Data profiles»: la lápida y `canonical`;
  - `clawg projects merge|keep-apart` en «Commands», y la lista de `projects list`;
  - las dos rutas nuevas, `merged` en `GET /v1/projects` y los 409 nuevos en la API;
  - los tres chequeos en `clawg doctor`.
- [ ] El contrato del modelo no cambia: `the_agent_contract_matches_its_file` y su variante en español pasan sin bendecir.
- Commit: `docs: record merging projects`

### 21. Verificación

- [ ] La suite y los dos clippy de `.claude/CLAUDE.md`, en `target-test` en Windows, con `--locked`.
- [ ] El binario en un home temporal, con repositorios reales:
  - dos carpetas registradas como proyectos distintos, con memorias, un remoto y un `workspace.md`, se unen; `projects list` muestra sólo el destino, con la lápida debajo;
  - `memories list --here` desde la carpeta de la lápida lee el destino, y un save hecho con el id viejo cae en el destino;
  - `projects merge --undo N` deja todo como estaba;
  - `keep-apart` sobre una copia saca su sugerencia, y registrarla otra vez no la vuelve a crear;
  - `doctor` nombra dos proyectos con el mismo nombre y propone la unión en seco.
- [ ] `verify.md`, sección «Entrega 2a».
- [ ] No queda otro directorio de build que `target` y `target-test`.
