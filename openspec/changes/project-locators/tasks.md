# project-locators — Tareas

Strict TDD: en cada tarea, el test va primero y falla, por la aserción o porque no compila. Cada tarea es un commit convencional que compila y pasa la suite (`cargo test --workspace --target-dir target-test` en Windows) y clippy (`cargo clippy --workspace --all-targets --target-dir target-test -- -A clippy::all -D warnings`). Antes de empezar una entrega: `cargo clean --workspace --target-dir target-test`. Las tareas de las entregas 3 y 4 se escriben cuando les toque. Ninguna entrega sube la versión: el cambio sale entero en 0.64.0.

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
  - `a_merge_is_refused_while_a_save_of_its_projects_is_pending`, `a_merge_of_a_tombstone_into_its_target_repairs_it` y `a_merge_is_refused_for_an_unknown_the_same_or_a_merged_project`;
  - la pura `decide_merge_carrying_takes_the_joined_part_of_a_unit_elsewhere` (`relocate.rs`).
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
  - un ayudante del daemon que resuelve un `Option<String>` de proyecto con `canonical`, en `save_memory`, `search`, `context`, `register_session`, `synthesis`, `disputes`, `list_memories` y `stats`;
  - en el core, para que valga para cualquier llamador: `MoveError::Merged` para un `from_project` que es una lápida, `MoveError::PartOfMerge` para el `--undo` del movimiento de una unión (`undo_move_core`) y `RenameError::Merged` para renombrar una lápida;
  - el daemon los responde 409 `project_merged` y 409 `part_of_merge`.
- Commit: `fix(daemon): follow a merged project at every entry that takes one`

### 16. La lista y `keep-apart` en el store (PL15, PL16; D22)

- [ ] RED:
  - `a_merged_project_is_listed_under_its_target_and_frees_its_name` y `keeping_two_projects_apart_answers_their_suggestion_for_good` (`registry.rs`);
  - `a_pair_kept_apart_is_not_suggested_again` (`project.rs`).
- [ ] GREEN:
  - `list_projects` sin lápidas, con `merged` y las sugerencias por `canonical`;
  - el nombre de una lápida, libre para `rename_project`;
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

## Entrega 2b. Borrar proyectos (V29)

Requisitos en la [spec](spec.md#entrega-2b-borrar-proyectos) (PL21 a PL31) y decisiones en el [diseño](design.md#entrega-2b-borrar-proyectos) (D26 a D37). Antes de la tarea 22: `cargo clean --workspace --target-dir target-test`.

### 22. V29 y la vuelta al uso (PL21, PL22; D26, D27, D28)

- [ ] RED:
  - `a_database_from_the_previous_version_opens_with_no_project_deleted` (`db.rs`, sembrado con `seed_on_disk(dir, Some(28))` y el escenario de PL21);
  - `a_live_row_brings_a_deleted_project_back_unreviewed` (por un insert, por un cambio de proyecto y por una restauración) y `a_deleted_row_or_a_registration_leaves_it_deleted` (`crates/clawg-core/src/memory/project_delete.rs`, con `deleted_at` escrito a mano);
  - la fila 29 en `published_migration_checksums_are_pinned` (`migrations.rs`).
- [ ] GREEN:
  - `crates/clawg-core/migrations/V29__project_delete.sql`: la columna, las tres tablas con sus índices, los dos disparadores reemplazados y `memory_project_restored`;
  - tocar `migrations.rs` en el mismo commit;
  - el módulo `project_delete.rs`, registrado en `memory/mod.rs`.
- Commit: `feat(core): add the mark and the journal of a deleted project`

### 23. La regla de los duplicados pasa al core (D36)

- [ ] RED: `duplicates_pair_by_name_and_by_first_commit_and_skip_answered_pairs` (`crates/clawg-core/src/memory/inspect.rs`), sobre los hechos de `inspect`.
- [ ] GREEN: la función pura `duplicates(registry, folders, answered)` en `inspect.rs`. `project_duplicates` (`crates/clawg-cli/src/doctor/checks.rs`) la llama, y sus tests pasan sin cambios.
- Commit: `refactor(core): move the rule of duplicate projects into the core`

### 24. Borrar un proyecto vacío, el informe y el código (PL23, PL24; D29, D31, D32, D36)

- [ ] RED (`project_delete.rs`, sobre `MemoryStore::new_for_test()`):
  - `an_empty_project_is_deleted_and_its_docs_index_goes`;
  - `a_project_with_live_rows_is_refused_with_its_candidates`;
  - `a_delete_is_refused_for_an_unknown_a_merged_or_a_deleted_project` y `a_delete_waits_for_the_saves_of_its_project`;
  - `the_report_counts_by_type_author_and_status_and_writes_nothing` y `the_code_changes_with_a_save_an_edit_a_restore_or_a_locator`;
  - la pura `the_code_of_a_set_does_not_depend_on_its_order`.
- [ ] GREEN:
  - `delete_core(store, workspaces, backups, project_id, with_memories, confirm)`, con:
    - la pasada que lee el conjunto (`set_in`);
    - el código puro (`code_of`);
    - los candidatos;
    - la espera de `SAVES_WAIT` para X y sus lápidas;
  - `DeleteReport`, y `DeleteError` con `thiserror`;
  - `drop_root_index_in(tx, root_id)` en `project_docs/mod.rs`, sobre `delete_doc`;
  - `WorkspaceManager::project_files(project_id)` (`workspace.rs`), con el recorrido de `adopt_legacy`.
- Commit: `feat(core): delete an empty project and report what a project holds`

### 25. Borrar con sus memorias (PL25; D29, D30)

- [ ] RED (`project_delete.rs`): `deleting_with_memories_marks_every_row_and_revives_none`, `a_delete_keeps_outside_rows_replaced_and_releases_their_disputes`, `a_stale_code_is_refused_with_the_new_report` y `a_delete_with_memories_waits_for_the_saves_of_its_project`.
- [ ] GREEN:
  - `confirm` en `delete_core`: el código en las dos pasadas, el snapshot, el marcado conjunto con sus vectores, `release_challengers` y el diario;
  - `Reason::PreDelete` (`backup.rs`, en `ALL`, `as_str` y los tests de la poda).
- Commit: `feat(core): delete a project with its memories under a confirmed code`

### 26. Deshacer un borrado (PL26; D33)

- [ ] RED (`project_delete.rs`): `undoing_a_delete_leaves_the_store_as_it_was`, `undoing_a_delete_keeps_what_was_saved_since`, `a_delete_followed_by_another_is_undone_after_it` y `undoing_an_empty_delete_needs_no_embedder`.
- [ ] GREEN:
  - `undo_delete_core(store, embedder, backups, delete_id, dry_run)`, con el embedder como `Option`. Sin él, un `--undo` con filas que restaurar responde `DeleteError::NeedsModel`;
  - la pasada que decide (negativas y unidades), las tandas de `embed_batch`, `pre-undo` y la transacción que vuelve a decidir;
  - `RESTORE_TEXT_INDEX` (`admin.rs:573`) pasa a `pub(crate)`.
- Commit: `feat(core): undo the delete of a project`

### 27. Lo que administra proyectos no toma uno borrado, y la lista (PL27; D35)

- [ ] RED:
  - `a_deleted_project_is_refused_by_what_manages_projects`: `rename_project` y `keep_apart` (`registry.rs`), los dos lados de una unión y el `--undo` de una con el destino borrado (`merge.rs`), y un movimiento o un `PATCH` hacia él (`relocate.rs`);
  - `a_deleted_project_is_listed_only_in_deletes_and_frees_its_name` (`registry.rs`).
- [ ] GREEN:
  - `RenameError::Deleted`, `MergeError::Deleted` y `MoveError::Deleted`, en `rename_project`, `keep_apart`, `merge_pass`, `undo_pass` y `target_in`;
  - `list_projects` sin los borrados, con `deletes` (`ListedDelete`) y el nombre libre.
- Commit: `feat(core): keep a deleted project out of what manages projects`

### 28. Borrar y deshacer en el daemon (PL22 a PL27)

- [ ] RED (`tests/api.rs`):
  - `deleting_through_the_daemon_takes_two_steps` y `undoing_a_delete_needs_the_model`;
  - `a_bridge_opened_before_a_delete_brings_its_project_back`;
  - `every_route_that_takes_a_project_treats_a_deleted_one_by_its_rule`;
  - `POST /projects/delete` en la tabla de `every_route_says_which_project_it_takes`, y su brazo en `every_route_that_takes_a_project_follows_a_merge`.
- [ ] GREEN:
  - `POST /v1/projects/delete` detrás del token, con el mapeo de `DeleteError`. Para `NeedsModel`, el de `ready_embedder`;
  - `deletes` en `GET /v1/projects`;
  - el 409 `project_deleted` en `rename`, `keep_projects_apart`, `merge_projects`, `move_memories` y el `PATCH` de una memoria.
- Commit: `feat(daemon): delete projects and undo their delete`

### 29. `clawg projects delete` (PL29)

- [ ] RED:
  - `projects_delete_parses` (`cli.rs`);
  - `a_delete_report_names_what_goes_and_the_exact_confirm`, `a_refused_delete_names_the_ways_to_empty_it` y `a_listing_ends_with_the_open_deletes` (`projects.rs`, en español y en inglés);
  - `a_project_is_deleted_and_undone_through_the_daemon` (`projects.rs`).
- [ ] GREEN: `ProjectsAction::Delete(DeleteOpts)`, el método del cliente, `render_delete`, `delete_refused` y el cierre de `render_list`, con una sentencia por `match` sobre `Lang`, y la salida 1 para `refused`.
- Commit: `feat(cli): add projects delete`

### 30. Purgar un borrado en el store (PL28; D34)

- [ ] RED (`crates/clawg-core/src/memory/secrets.rs`): `purging_a_delete_leaves_no_trace_of_its_rows`, `purging_a_delete_keeps_a_row_restored_since`, `a_purge_of_a_delete_undone_or_purged_is_refused_untouched` y `purging_a_delete_names_the_backups_and_workspaces_that_hold_its_text`.
- [ ] GREEN: `purge_delete(base_dir, delete_id) -> Result<PurgedDelete, MemoryError>`, con:
  - `open_to_rewrite`;
  - una transacción;
  - una sola pasada por los backups y los `workspace.md`;
  - `leave_no_old_copy`.
- Commit: `feat(core): purge for good the rows a project delete left`

### 31. `clawg purge --project-delete` (PL28)

- [ ] RED:
  - `purge_takes_an_id_or_a_delete` (`cli.rs`);
  - `the_purge_report_of_a_delete_names_what_went` (`crates/clawg-cli/src/secrets.rs`, en los dos idiomas);
  - `a_deleted_project_is_purged_once_the_daemon_stops` (`tests/binary.rs`): un daemon con `--port 0` y el embedder `fake`, un proyecto con memorias, el borrado en dos pasos, `clawg stop` y la purga.
- [ ] GREEN: `Command::Purge` con `<ID>` o `--project-delete N` en un grupo requerido, `confirm` para los dos y `purge_delete_report`.
- Commit: `feat(cli): purge what a project delete left`

### 32. Los hallazgos de `doctor` (PL30; D37)

- [ ] RED: `a_deleted_project_is_no_duplicate_nor_orphan` y `live_rows_under_a_deleted_project_are_named` (`checks.rs`); la lectura de los hechos nuevos en un store en V28 y en V29 (`inspect.rs`).
- [ ] GREEN:
  - `deleted_at` en `RegistryRow`, y los conteos de filas vivas en `IdRows`;
  - `project_deletes` en `checks.rs`, con sus mensajes en los dos idiomas;
  - su llamada en `doctor/mod.rs`, y su lugar en `every_finding`.
- Commit: `feat(doctor): name live rows under a deleted project`

### 33. Las specs y la documentación

- [ ] `specs/project`: PL21 a PL31, y lo que cambia PL27 en PL9 y PL15.
- [ ] `specs/storage`: V29, su vuelta atrás, el motivo `pre-delete` y la poda.
- [ ] `specs/secrets`: R5 con `--project-delete`.
- [ ] `specs/restore`: `undelete` hace volver al proyecto borrado de su fila.
- [ ] `specs/doctor`: `project_deletes`, y lo que cambia en `project_duplicates`.
- [ ] `.claude/CLAUDE.md`:
  - el párrafo del proyecto en «Data profiles»: un proyecto borrado, su vuelta al uso y su diario;
  - `clawg projects delete` y `clawg purge --project-delete` en «Commands», y el cierre de `projects list`;
  - la ruta nueva, `deletes` en `GET /v1/projects` y el 409 `project_deleted` en la API;
  - `project_deletes` en `clawg doctor`.
- [ ] El contrato del modelo no cambia: `the_agent_contract_matches_its_file` y su variante en español pasan sin volver a bendecir.
- Commit: `docs: record deleting projects`

### 34. Verificación

- [ ] La suite y los dos clippy de `.claude/CLAUDE.md`, en `target-test` en Windows, con `--locked`.
- [ ] El binario en un home temporal, con repositorios reales:
  - un proyecto vacío se borra, deja de listarse, y su `--undo` lo devuelve;
  - uno con memorias se rechaza con sus candidatos;
  - el informe da un código, un save entre los dos pasos lo cambia, y el código nuevo borra;
  - un save desde un puente abierto en su carpeta lo hace volver, y lo borrado sigue borrado;
  - `projects delete --undo N` restaura las memorias;
  - con el daemon parado, `purge --project-delete N --yes` no deja sus textos en el archivo y nombra el snapshot `pre-delete`;
  - `doctor` no marca nada nuevo.
- [ ] `verify.md`, sección «Entrega 2b».
- [ ] No queda otro directorio de build que `target` y `target-test`.
