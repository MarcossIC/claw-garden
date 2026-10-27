# project-locators — Tareas

Strict TDD: en cada tarea, el test va primero y falla, por la aserción o porque no compila. Cada tarea es un commit convencional que compila y pasa la suite (`cargo test --workspace --target-dir target-test` en Windows) y clippy (`cargo clippy --workspace --all-targets --target-dir target-test -- -A clippy::all -D warnings`). Antes de empezar una entrega: `cargo clean --workspace --target-dir target-test`. Ninguna entrega sube la versión: el cambio sale entero en 0.64.0.

## Entrega 1. El registro (V27)

Requisitos en la [spec](spec.md) (PL1 a PL10) y decisiones en el [diseño](design.md) (D1 a D14).

### 1. V27: `project` como referencia, `project_path` para las carpetas, y la adopción (PL1, PL2, PL8; D1, D6, D7)

- [x] RED:
  - `a_database_from_the_previous_version_opens_with_a_project_for_every_id_in_use` (`db.rs`, sembrado con `seed_on_disk(dir, Some(26))` y el escenario de PL1);
  - `a_row_saved_under_an_unknown_project_adopts_it`, `a_row_moved_to_an_unknown_project_adopts_it` y `the_name_of_an_adopted_id_is_the_one_rust_gives` (`crates/clawg-core/src/memory/registry.rs`);
  - `a_project_without_a_folder_is_a_target` (`relocate.rs`);
  - la fila 27 en `published_migration_checksums_are_pinned` (`migrations.rs`).
- [x] GREEN:
  - `crates/clawg-core/migrations/V27__project_registry.sql`: el renombre, las columnas de `project_path`, las tablas `project` y `project_suggestion`, los índices, el relleno y los dos disparadores;
  - tocar `migrations.rs` en el mismo commit, para que se recompile;
  - `registry.rs` con `project_name`;
  - cada lector y escritor de carpetas pasa a `project_path`: `project.rs`, `relocate.rs` (salvo `check_target_in`, que sigue en `project`), `project_docs/mod.rs`, `import.rs`, `undo.rs`;
  - los que abren stores sin migrar eligen la tabla según el esquema: `inspect.rs`, `export.rs` (`read` y `project_id_of`);
  - los tests que insertan o cuentan carpetas pasan a `project_path`. Los de migraciones anteriores las leen de `project_path` después de abrir, sin cambiar lo que comprueban.
- Commit: `feat(core): make project the reference of every project and keep folders in project_path`

### 2. Las huellas de un checkout (PL3; D3)

- [x] RED:
  - en `crates/clawg-core/tests/project_root.rs`, con repositorios reales creados con `git` en directorios temporales: `the_first_commit_is_the_root_of_the_first_parent_chain` (un merge de una historia ajena no lo cambia), `a_shallow_or_empty_repository_has_no_first_commit_to_record` (un `clone --depth 1` desde `file://`, y un `git init` sin commits) y `the_stamp_of_a_checkout_changes_when_it_is_cloned_again`;
  - `a_command_that_does_not_end_in_time_is_stopped` (`identity.rs`, con un `ping` o un `sleep`, como el test de `serve.rs`).
- [x] GREEN:
  - en `identity.rs`: `git_stamp`, `root_commit` (con el resultado de tres casos de PL3), la espera con plazo, `CREATE_NO_WINDOW` en Windows y el entorno sin `GIT_*`;
  - el trait `Ground` y su versión real, `Disk`.
- Commit: `feat(core): read the first commit and the git directory of a checkout`

### 3. Una carpeta sirve a una raíz sólo si sus huellas no la contradicen (PL3, PL4, PL7; D4, D9, D13)

- [x] RED:
  - la pura `a_checkout_settles_on_the_folder_its_fingerprints_do_not_contradict`;
  - sobre `MemoryStore::new_for_test()` con un `Ground` falso que cuenta las lecturas: `a_recorded_checkout_does_not_read_its_first_commit_again`, `a_repository_without_origin_in_the_path_of_another_gets_a_project_of_its_own`, `two_repositories_without_origin_in_one_path_are_two_projects`, `a_remote_decides_before_the_first_commit`, `a_clone_folder_records_its_own_remote` y `a_caller_that_derives_its_id_is_refused_a_repository_without_origin_in_its_path` (`project.rs`);
  - `a_repository_without_origin_replaced_in_its_path_registers_into_a_project_of_its_own` (`crates/clawg-cli/tests/api.rs`, con repositorios reales);
  - los tests de R4 y de R14 a R20 de `project.rs` pasan a usar el `Ground` falso y siguen pasando sin cambiar lo que comprueban.
- [x] GREEN:
  - `fit` y `settle` sobre huellas, y `path_rows_in` con las columnas nuevas;
  - la pasada que pide lo que falta, y `Registration::checkout`;
  - `write_in` graba `project_id`, `remote`, `root_commit` y `git_stamp`, y crea el proyecto `root` que falte;
  - `register` lee las huellas con `Disk`;
  - `project_id_in`, `plan_ids_in` y `project_id_of` con `Ground`, y sus llamadores en `memories.rs` y `main.rs`.
- Commit: `fix(core): tell two repositories in one path apart by their first commit`

### 4. Mudanzas y sugerencias (PL5, PL6, PL7; D5, D10)

- [x] RED:
  - `a_checkout_moved_without_origin_keeps_its_project`, `a_copy_of_a_checkout_is_a_project_of_its_own_with_a_suggestion`, `a_checkout_two_projects_could_be_is_a_project_of_its_own`, `a_renamed_remote_in_its_path_is_another_project_with_a_suggestion`, `a_suggestion_is_made_once_per_pair`, `a_moved_checkout_is_read_and_planned_alike_without_writing` y `a_caller_that_derives_its_id_never_follows_a_move` (`project.rs`);
  - `a_folder_moved_without_origin_registers_into_its_project` (`tests/api.rs`, con repositorios reales).
- [x] GREEN:
  - `moved` (pura);
  - en la pasada, leer los checkouts grabados con ese primer commit;
  - la carpeta nueva vinculada, las sugerencias (`INSERT OR IGNORE`) y sus líneas de log.
- Commit: `feat(core): keep the project of a folder moved without origin`

### 5. El import escribe sus carpetas como un registro (PL7; D12)

- [x] RED: `an_import_records_its_folders_as_a_registration_would` (`import.rs`). Comprueba tres cosas:
  - una raíz del archivo que es la mudanza de un checkout de aquí manda sus filas al proyecto del plan, y su carpeta lo dice;
  - una raíz con remoto graba ese remoto;
  - el proyecto que no existía queda en `project` con `born = 'import'`.
- [x] GREEN:
  - `roots_here` lee las huellas en un hilo bloqueante;
  - `record_projects` inserta las columnas nuevas y el proyecto;
  - el import no crea sugerencias.
- Commit: `fix(core): record an imported folder as a registration would`

### 6. Listar y renombrar en el daemon (PL9; D8, D11)

- [x] RED:
  - `renaming_keeps_the_id_and_marks_the_project_reviewed`, `a_name_another_project_holds_is_refused` y `a_name_is_trimmed_and_bounded` (`registry.rs`);
  - `the_projects_are_listed_with_their_folders_and_suggestions` (con un proyecto sin carpeta y una carpeta que ya no está), `renaming_a_project_keeps_what_was_saved_under_it` y `renaming_an_unknown_project_is_404` (`tests/api.rs`).
- [x] GREEN:
  - `registry::list` y `registry::rename`;
  - `GET /v1/projects` y `PATCH /v1/projects/{id}` detrás del token, con la clave y la presencia de cada carpeta leídas en un hilo bloqueante.
- Commit: `feat(daemon): list and rename projects`

### 7. `clawg projects list` y `clawg projects rename` (PL9)

- [x] RED: `projects_list_and_rename_parse` (`cli.rs`); `a_listing_marks_what_was_created_alone_and_the_suggestions` (`projects.rs`, en español y en inglés).
- [x] GREEN: `ProjectsAction::List` y `ProjectsAction::Rename`, los métodos del cliente y la impresión, con una sentencia por `match` sobre `Lang`.
- Commit: `feat(cli): add projects list and rename`

### 8. Las specs y la documentación

- [x] `specs/project`:
  - PL1 a PL10;
  - R10, R16, R18 y R20 al día;
  - las definiciones de proyecto y carpeta;
  - el «Origen».
- [x] `specs/storage`: V27 (el renombre, el relleno, los disparadores) y su vuelta atrás.
- [x] `specs/doctor`: la regla `projects` lee las carpetas de `project_path`.
- [x] `.claude/CLAUDE.md`:
  - el párrafo del proyecto en «Data profiles»: proyecto y carpeta, el primer commit, las mudanzas y las sugerencias;
  - `clawg projects list|rename` en «Commands»;
  - las dos rutas nuevas en la API.
- [x] El contrato del modelo no cambia: `the_agent_contract_matches_its_file` pasa sin bendecir.
- Commit: `docs: record projects as the reference and folders as one way to reach them`

### 9. La versión

- [x] Si 0.63.0 ya se publicó: `0.63.0` → `0.64.0` (la entrega se publica sola y trae esquema y comandos nuevos), y `cargo update --workspace`. Si no se publicó, la entrega sale con la 0 de 89 en 0.63.0 y la versión no sube (como pasó al [archivar 89](../2026-10-25-project-registry/tasks.md)).
- Commit: `chore: bump version to 0.64.0`

### 10. Verificación

- [x] La suite y clippy de `.claude/CLAUDE.md`, en `target-test` en Windows.
- [x] El binario en un home temporal, con repositorios reales:
  - una carpeta sin `origin` registrada, movida y registrada de nuevo conserva su proyecto;
  - otra en su camino viejo recibe uno propio;
  - `projects list` muestra la sugerencia de una copia y un proyecto cuya carpeta se borró;
  - `projects rename` conserva lo guardado.
- [x] El CI en Linux y Windows: el de los pull requests #95, #96 y #97 a `develop`, sobre lo unido hasta cada uno. El de `develop` a `main` lo vuelve a correr antes del release.
- [x] `verify.md`, sección «Entrega 1».
- [x] No queda otro directorio de build que `target` y `target-test`.

## Entrega 2a. Unir proyectos (V28)

Requisitos en la [spec](spec.md#entrega-2a-unir-proyectos) (PL11 a PL20) y decisiones en el [diseño](design.md#entrega-2a-unir-proyectos) (D15 a D25). Antes de la tarea 11: `cargo clean --workspace --target-dir target-test`.

### 11. V28 y `canonical` (PL11, PL12; D15, D16)

- [x] RED:
  - `a_database_from_the_previous_version_opens_with_every_project_live` (`db.rs`, sembrado con `seed_on_disk(dir, Some(27))` y el escenario de PL11);
  - `canonical_follows_a_merge_and_leaves_any_other_id_as_it_is` y `a_cycle_or_a_chain_too_long_answers_the_id_itself` (`crates/clawg-core/src/memory/merge.rs`, con lápidas escritas a mano);
  - la fila 28 en `published_migration_checksums_are_pinned` (`migrations.rs`).
- [x] GREEN:
  - `crates/clawg-core/migrations/V28__project_merge.sql`: las dos columnas, el índice parcial, `project_merge` y `project_merge_change`;
  - tocar `migrations.rs` en el mismo commit;
  - `merge.rs` con `canonical_in(c, id)` y `MAX_MERGE_HOPS`.
- Commit: `feat(core): add the tombstone of a merged project`

### 12. Unir X en Z en el store (PL13; D17 a D21)

- [x] RED (`merge.rs`, sobre `MemoryStore::new_for_test()`):
  - `merging_moves_every_row_and_locator_into_the_target`;
  - `a_merge_keeps_the_newest_save_of_a_key_and_the_oldest_of_a_text`;
  - `a_merge_moves_the_part_of_a_split_unit_it_holds`;
  - `merging_a_target_repoints_its_tombstones`;
  - `a_dry_run_merge_changes_nothing`;
  - `a_merge_copies_a_workspace_only_where_the_target_has_none`;
  - `a_merge_is_refused_while_a_save_of_its_projects_is_pending`, `a_merge_of_a_tombstone_into_its_target_repairs_it` y `a_merge_is_refused_for_an_unknown_the_same_or_a_merged_project`;
  - la pura `decide_merge_carrying_takes_the_joined_part_of_a_unit_elsewhere` (`relocate.rs`).
- [x] GREEN:
  - `Span::{Keep, Carry}` en `decide_merge` (`relocate.rs`): el registro pasa `Keep`;
  - `merge_core(store, workspaces, backups, source, target, dry_run)`: la pasada en seco, la espera de `SAVES_WAIT`, el snapshot, la transacción que vuelve a decidir y el diario;
  - `MergeError` con `thiserror`;
  - `Reason::PreMerge` (`backup.rs`);
  - `adopt_legacy` devuelve `Adopted { copied, kept_beside }`, y el registro lo ignora.
- Commit: `feat(core): merge one project into another`

### 13. Deshacer una unión en el store (PL14; D20)

- [x] RED (`merge.rs`): `undoing_a_merge_leaves_the_store_as_it_was`, `undoing_a_merge_keeps_what_was_saved_after_it`, `a_merge_whose_target_was_merged_since_is_undone_after_that_one` y `a_workspace_edited_since_the_merge_is_kept`.
- [x] GREEN:
  - el cuerpo de `undo_move_core` pasa a `undo_move_in(c, move_id, write)`, que la unión llama dentro de su transacción. `undo_move_core` sigue igual por fuera;
  - `undo_merge_core`, con `pre-undo` y el borrado de copias por la parte del usuario.
- Commit: `feat(core): undo a merge of projects`

### 14. Los escritores siguen la lápida (PL12; D17)

- [x] RED:
  - `a_save_retried_after_a_merge_lands_in_its_target` (`worker.rs`);
  - `a_move_to_a_merged_project_lands_in_its_target` y `undoing_a_move_whose_origin_was_merged_returns_to_its_target` (`relocate.rs`);
  - `a_root_whose_project_was_merged_registers_into_its_target`, también leyendo y planeando sin escribir (`project.rs`);
  - `an_import_planned_before_a_merge_writes_into_its_target` (`import.rs`).
- [x] GREEN:
  - `canonical_in` al empezar `process_tracked` (`worker.rs:676`);
  - `check_target_in` devuelve el destino vivo, y el `--undo` de un movimiento resuelve el origen de cada fila;
  - `plan_in` y `plan_ids_in` resuelven el proyecto que reciben, después de `project_of` y de una mudanza;
  - el import resuelve el proyecto de cada fila en la transacción que la escribe.
- Commit: `fix(core): write under the target of a merged project`

### 15. Las entradas del daemon siguen la lápida (PL12, PL19; D23, D24)

- [x] RED (`tests/api.rs`, con una unión hecha en el store del `Harness`):
  - `every_route_says_which_project_it_takes`, que lee `router()` de `api.rs`;
  - `every_route_that_takes_a_project_follows_a_merge`;
  - `a_bridge_opened_before_a_merge_reads_and_writes_its_target`;
  - `a_move_from_a_merged_project_is_refused` y `the_move_of_a_merge_is_undone_only_with_its_merge`.
- [x] GREEN:
  - un ayudante del daemon que resuelve un `Option<String>` de proyecto con `canonical`, en `save_memory`, `search`, `context`, `register_session`, `synthesis`, `disputes`, `list_memories` y `stats`;
  - en el core, para que valga para cualquier llamador: `MoveError::Merged` para un `from_project` que es una lápida, `MoveError::PartOfMerge` para el `--undo` del movimiento de una unión (`undo_move_core`) y `RenameError::Merged` para renombrar una lápida;
  - el daemon los responde 409 `project_merged` y 409 `part_of_merge`.
- Commit: `fix(daemon): follow a merged project at every entry that takes one`

### 16. La lista y `keep-apart` en el store (PL15, PL16; D22)

- [x] RED:
  - `a_merged_project_is_listed_under_its_target_and_frees_its_name` y `keeping_two_projects_apart_answers_their_suggestion_for_good` (`registry.rs`);
  - `a_pair_kept_apart_is_not_suggested_again` (`project.rs`).
- [x] GREEN:
  - `list_projects` sin lápidas, con `merged` y las sugerencias por `canonical`;
  - el nombre de una lápida, libre para `rename_project`;
  - `keep_apart`.
- Commit: `feat(core): list a merged project under its target and keep two projects apart`

### 17. Unir y separar en el daemon (PL13, PL14, PL16)

- [x] RED (`tests/api.rs`): `merging_through_the_daemon_leaves_everything_under_the_target`, `a_merge_waits_for_the_saves_of_its_projects` y `the_listing_after_a_merge_shows_the_target_alone`. Las dos rutas nuevas entran a la tabla de `every_route_says_which_project_it_takes`.
- [x] GREEN: `POST /v1/projects/merge` y `POST /v1/projects/keep-apart` detrás del token, con el mapeo de `MergeError` y de los rechazos de PL13, PL14 y PL16.
- Commit: `feat(daemon): merge projects and keep them apart`

### 18. `clawg projects merge` y `clawg projects keep-apart` (PL17)

- [x] RED: `projects_merge_and_keep_apart_parse` (`cli.rs`); `a_merge_report_names_what_moved_and_the_way_back` y `a_listing_names_what_was_merged_into_a_project` (`projects.rs`, en español y en inglés).
- [x] GREEN: `ProjectsAction::Merge` y `ProjectsAction::KeepApart`, los métodos del cliente, `render_merge` y la línea de `render_list`, con una sentencia por `match` sobre `Lang`.
- Commit: `feat(cli): add projects merge and keep-apart`

### 19. Los hallazgos de `doctor` (PL18; D25)

- [x] RED: `the_duplicates_are_named_by_name_and_by_first_commit`, `a_pair_kept_apart_is_no_duplicate`, `an_id_without_a_project_is_named` y `rows_under_a_merged_project_and_a_broken_chain_are_named` (`checks.rs`); la lectura de los hechos nuevos en un store en V26, en V27 y en V28 (`inspect.rs`).
- [x] GREEN: los hechos en `inspect.rs`, los tres chequeos en `checks.rs` con sus mensajes en los dos idiomas, su llamada en `doctor/mod.rs` y su lugar en `every_finding`.
- Commit: `feat(doctor): name duplicate projects, ids without a project and broken merges`

### 20. Las specs y la documentación

- [x] `specs/project`: PL11 a PL20, y PL9 con lo que cambia PL15.
- [x] `specs/storage`: V28, su vuelta atrás y el motivo `pre-merge`.
- [x] `specs/doctor`: los tres chequeos.
- [x] `.claude/CLAUDE.md`:
  - el párrafo del proyecto en «Data profiles»: la lápida y `canonical`;
  - `clawg projects merge|keep-apart` en «Commands», y la lista de `projects list`;
  - las dos rutas nuevas, `merged` en `GET /v1/projects` y los 409 nuevos en la API;
  - los tres chequeos en `clawg doctor`.
- [x] El contrato del modelo no cambia: `the_agent_contract_matches_its_file` y su variante en español pasan sin bendecir.
- Commit: `docs: record merging projects`

### 21. Verificación

- [x] La suite y los dos clippy de `.claude/CLAUDE.md`, en `target-test` en Windows, con `--locked`.
- [x] El binario en un home temporal, con repositorios reales:
  - dos carpetas registradas como proyectos distintos, con memorias, un remoto y un `workspace.md`, se unen; `projects list` muestra sólo el destino, con la lápida debajo;
  - `memories list --here` desde la carpeta de la lápida lee el destino, y un save hecho con el id viejo cae en el destino;
  - `projects merge --undo N` deja todo como estaba;
  - `keep-apart` sobre una copia saca su sugerencia, y registrarla otra vez no la vuelve a crear;
  - `doctor` nombra dos proyectos con el mismo nombre y propone la unión en seco.
- [x] `verify.md`, sección «Entrega 2a».
- [x] No queda otro directorio de build que `target` y `target-test`.

## Entrega 2b. Borrar proyectos (V29)

Requisitos en la [spec](spec.md#entrega-2b-borrar-proyectos) (PL21 a PL31) y decisiones en el [diseño](design.md#entrega-2b-borrar-proyectos) (D26 a D37). Antes de la tarea 22: `cargo clean --workspace --target-dir target-test`.

### 22. V29 y la vuelta al uso (PL21, PL22; D26, D27, D28)

- [x] RED:
  - `a_database_from_the_previous_version_opens_with_no_project_deleted` (`db.rs`, sembrado con `seed_on_disk(dir, Some(28))` y el escenario de PL21);
  - `a_live_row_brings_a_deleted_project_back_unreviewed` (por un insert, por un cambio de proyecto y por una restauración) y `a_deleted_row_or_a_registration_leaves_it_deleted` (`crates/clawg-core/src/memory/project_delete.rs`, con `deleted_at` escrito a mano);
  - la fila 29 en `published_migration_checksums_are_pinned` (`migrations.rs`).
- [x] GREEN:
  - `crates/clawg-core/migrations/V29__project_delete.sql`: la columna, las tres tablas con sus índices, los dos disparadores reemplazados y `memory_project_restored`;
  - tocar `migrations.rs` en el mismo commit;
  - el módulo `project_delete.rs`, registrado en `memory/mod.rs`.
- Commit: `feat(core): add the mark and the journal of a deleted project`

### 23. La regla de los duplicados pasa al core (D36)

- [x] RED: `duplicates_pair_by_name_and_by_first_commit_and_skip_answered_pairs` (`crates/clawg-core/src/memory/inspect.rs`), sobre los hechos de `inspect`.
- [x] GREEN: la función pura `duplicates(registry, folders, answered)` en `inspect.rs`. `project_duplicates` (`crates/clawg-cli/src/doctor/checks.rs`) la llama, y sus tests pasan sin cambios.
- Commit: `refactor(core): move the rule of duplicate projects into the core`

### 24. Borrar un proyecto vacío, el informe y el código (PL23, PL24; D29, D31, D32, D36)

- [x] RED (`project_delete.rs`, sobre `MemoryStore::new_for_test()`):
  - `an_empty_project_is_deleted_and_its_docs_index_goes`;
  - `a_project_with_live_rows_is_refused_with_its_candidates`;
  - `a_delete_is_refused_for_an_unknown_a_merged_or_a_deleted_project` y `a_delete_waits_for_the_saves_of_its_project`;
  - `the_report_counts_by_type_author_and_status_and_writes_nothing` y `the_code_changes_with_a_save_an_edit_a_restore_or_a_locator`;
  - la pura `the_code_of_a_set_does_not_depend_on_its_order`.
- [x] GREEN:
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

- [x] RED (`project_delete.rs`): `deleting_with_memories_marks_every_row_and_revives_none`, `a_delete_keeps_outside_rows_replaced_and_releases_their_disputes`, `a_stale_code_is_refused_with_the_new_report` y `a_delete_with_memories_waits_for_the_saves_of_its_project`.
- [x] GREEN:
  - `confirm` en `delete_core`: el código en las dos pasadas, el snapshot, el marcado conjunto con sus vectores, `release_challengers` y el diario;
  - `Reason::PreDelete` (`backup.rs`, en `ALL`, `as_str` y los tests de la poda).
- Commit: `feat(core): delete a project with its memories under a confirmed code`

### 26. Deshacer un borrado (PL26; D33)

- [x] RED (`project_delete.rs`): `undoing_a_delete_leaves_the_store_as_it_was`, `undoing_a_delete_keeps_what_was_saved_since`, `a_delete_followed_by_another_is_undone_after_it` y `undoing_an_empty_delete_needs_no_embedder`.
- [x] GREEN:
  - `undo_delete_core(store, embedder, backups, delete_id, dry_run)`, con el embedder como `Option`. Sin él, un `--undo` con filas que restaurar responde `DeleteError::NeedsModel`;
  - la pasada que decide (negativas y unidades), las tandas de `embed_batch`, `pre-undo` y la transacción que vuelve a decidir;
  - `RESTORE_TEXT_INDEX` (`admin.rs:573`) pasa a `pub(crate)`.
- Commit: `feat(core): undo the delete of a project`

### 27. Lo que administra proyectos no toma uno borrado, y la lista (PL27; D35)

- [x] RED:
  - `a_deleted_project_is_refused_by_what_manages_projects`: `rename_project` y `keep_apart` (`registry.rs`), los dos lados de una unión y el `--undo` de una con el destino borrado (`merge.rs`), y un movimiento o un `PATCH` hacia él (`relocate.rs`);
  - `a_deleted_project_is_listed_only_in_deletes_and_frees_its_name` (`registry.rs`).
- [x] GREEN:
  - `RenameError::Deleted`, `MergeError::Deleted` y `MoveError::Deleted`, en `rename_project`, `keep_apart`, `merge_pass`, `undo_pass` y `target_in`;
  - `list_projects` sin los borrados, con `deletes` (`ListedDelete`) y el nombre libre.
- Commit: `feat(core): keep a deleted project out of what manages projects`

### 28. Borrar y deshacer en el daemon (PL22 a PL27)

- [x] RED (`tests/api.rs`):
  - `deleting_through_the_daemon_takes_two_steps` y `undoing_a_delete_needs_the_model`;
  - `a_bridge_opened_before_a_delete_brings_its_project_back`;
  - `every_route_that_takes_a_project_treats_a_deleted_one_by_its_rule`;
  - `POST /projects/delete` en la tabla de `every_route_says_which_project_it_takes`, y su brazo en `every_route_that_takes_a_project_follows_a_merge`.
- [x] GREEN:
  - `POST /v1/projects/delete` detrás del token, con el mapeo de `DeleteError`. Para `NeedsModel`, el de `ready_embedder`;
  - `deletes` en `GET /v1/projects`;
  - el 409 `project_deleted` en `rename`, `keep_projects_apart`, `merge_projects`, `move_memories` y el `PATCH` de una memoria.
- Commit: `feat(daemon): delete projects and undo their delete`

### 29. `clawg projects delete` (PL29)

- [x] RED:
  - `projects_delete_parses` (`cli.rs`);
  - `a_delete_report_names_what_goes_and_the_exact_confirm`, `a_refused_delete_names_the_ways_to_empty_it` y `a_listing_ends_with_the_open_deletes` (`projects.rs`, en español y en inglés);
  - `a_project_is_deleted_and_undone_through_the_daemon` (`projects.rs`).
- [x] GREEN: `ProjectsAction::Delete(DeleteOpts)`, el método del cliente, `render_delete`, `delete_refused` y el cierre de `render_list`, con una sentencia por `match` sobre `Lang`, y la salida 1 para `refused`.
- Commit: `feat(cli): add projects delete`

### 30. Purgar un borrado en el store (PL28; D34)

- [x] RED (`crates/clawg-core/src/memory/secrets.rs`): `purging_a_delete_leaves_no_trace_of_its_rows`, `purging_a_delete_keeps_a_row_restored_since`, `a_purge_of_a_delete_undone_or_purged_is_refused_untouched` y `purging_a_delete_names_the_backups_and_workspaces_that_hold_its_text`.
- [x] GREEN: `purge_delete(base_dir, delete_id) -> Result<PurgedDelete, MemoryError>`, con:
  - `open_to_rewrite`;
  - una transacción;
  - una sola pasada por los backups y los `workspace.md`;
  - `leave_no_old_copy`.
- Commit: `feat(core): purge for good the rows a project delete left`

### 31. `clawg purge --project-delete` (PL28)

- [x] RED:
  - `purge_takes_an_id_or_a_delete` (`cli.rs`);
  - `the_purge_report_of_a_delete_names_what_went` (`crates/clawg-cli/src/secrets.rs`, en los dos idiomas);
  - `a_deleted_project_is_purged_once_the_daemon_stops` (`tests/binary.rs`): un daemon con `--port 0` y el embedder `fake`, un proyecto con memorias, el borrado en dos pasos, `clawg stop` y la purga.
- [x] GREEN: `Command::Purge` con `<ID>` o `--project-delete N` en un grupo requerido, `confirm` para los dos y `purge_delete_report`.
- Commit: `feat(cli): purge what a project delete left`

### 32. Los hallazgos de `doctor` (PL30; D37)

- [x] RED: `a_deleted_project_is_no_duplicate_nor_orphan` y `live_rows_under_a_deleted_project_are_named` (`checks.rs`); la lectura de los hechos nuevos en un store en V28 y en V29 (`inspect.rs`).
- [x] GREEN:
  - `deleted_at` en `RegistryRow`, y los conteos de filas vivas en `IdRows`;
  - `project_deletes` en `checks.rs`, con sus mensajes en los dos idiomas;
  - su llamada en `doctor/mod.rs`, y su lugar en `every_finding`.
- Commit: `feat(doctor): name live rows under a deleted project`

### 33. Las specs y la documentación

- [x] `specs/project`: PL21 a PL31, y lo que cambia PL27 en PL9 y PL15.
- [x] `specs/storage`: V29, su vuelta atrás, el motivo `pre-delete` y la poda.
- [x] `specs/secrets`: R5 con `--project-delete`.
- [x] `specs/restore`: `undelete` hace volver al proyecto borrado de su fila.
- [x] `specs/doctor`: `project_deletes`, y lo que cambia en `project_duplicates`.
- [x] `.claude/CLAUDE.md`:
  - el párrafo del proyecto en «Data profiles»: un proyecto borrado, su vuelta al uso y su diario;
  - `clawg projects delete` y `clawg purge --project-delete` en «Commands», y el cierre de `projects list`;
  - la ruta nueva, `deletes` en `GET /v1/projects` y el 409 `project_deleted` en la API;
  - `project_deletes` en `clawg doctor`.
- [x] El contrato del modelo no cambia: `the_agent_contract_matches_its_file` y su variante en español pasan sin volver a bendecir.
- Commit: `docs: record deleting projects`

### 34. Verificación

- [x] La suite y los dos clippy de `.claude/CLAUDE.md`, en `target-test` en Windows, con `--locked`.
- [x] El binario en un home temporal, con repositorios reales:
  - un proyecto vacío se borra, deja de listarse, y su `--undo` lo devuelve;
  - uno con memorias se rechaza con sus candidatos;
  - el informe da un código, un save entre los dos pasos lo cambia, y el código nuevo borra;
  - un save desde un puente abierto en su carpeta lo hace volver, y lo borrado sigue borrado;
  - `projects delete --undo N` restaura las memorias;
  - con el daemon parado, `purge --project-delete N --yes` no deja sus textos en el archivo y nombra el snapshot `pre-delete`;
  - `doctor` no marca nada nuevo.
- [x] `verify.md`, sección «Entrega 2b».
- [x] No queda otro directorio de build que `target` y `target-test`.

## Entrega 3a. Proyectos sin carpeta y el agente

Requisitos en la [spec](spec.md#entrega-3a-proyectos-sin-carpeta-y-el-agente) (PL32 a PL42) y decisiones en el [diseño](design.md#entrega-3a-proyectos-sin-carpeta-y-el-agente) (D38 a D49). Sin migración.

### 35. Resolver una referencia y crear un proyecto en el store (PL32, PL33; D38 a D43)

- [x] RED (`registry.rs`):
  - las puras `name_keys_join_what_people_write_alike` y `a_new_project_gets_a_slug_and_four_hex_digits`;
  - `a_reference_is_an_id_before_a_name`, `a_name_two_projects_hold_is_ambiguous`, `a_similar_name_creates_nothing_unless_asked`, `a_deleted_project_is_a_candidate_by_name_and_itself_by_id`, `a_reference_is_bounded` y `a_project_is_created_once_by_name`.
- [x] GREEN: `name_key`, `new_project_id`, `resolve_project(store, reference, ResolveOptions)` con `Resolution` y sus candidatos, `ResolveError` con `thiserror`, y `find_project` (`Named`) para lo que toma un proyecto que ya existe.
- Commit: `feat(core): resolve a project by id or name and create one without a folder`

### 36. La sugerencia por nombre de carpeta (PL38; D46)

- [x] RED (`project.rs`): `a_folder_named_as_a_project_whose_folders_are_gone_is_suggested` y `a_folder_named_as_a_project_still_there_is_not_suggested`, con el `Ground` falso.
- [x] GREEN: en `plan_in`, los proyectos de la misma clave de nombre con sus checkouts grabados entran a `Planned::Presence`, y la sugerencia `folder_name` a `Plan::suggestions`.
- Commit: `feat(core): suggest the project a moved folder is named after`

### 37. Las sugerencias del contexto y su respuesta en el store (PL36, PL37; D45)

- [x] RED (`registry.rs`): `the_pending_suggestions_of_a_project_are_read_through_merges` y `a_suggestion_is_answered_only_when_pending`.
- [x] GREEN: `pending_suggestions(store, project_id)`, con la regla de pares (`pending_pairs`) que ahora comparte `listed`; `suggestion_between(store, a, b)`, que revisa los dos proyectos como `keep_apart`.
- Commit: `feat(core): read the pending suggestions of a project`

### 38. Las rutas del daemon (PL32, PL35 a PL37, PL39, PL40)

- [x] RED (`tests/api.rs`):
  - `a_reference_resolves_through_the_daemon`;
  - `a_move_and_a_patch_take_a_project_by_name` y `a_name_two_projects_hold_is_refused_as_a_target`;
  - `the_context_names_the_pending_suggestions_of_its_project`;
  - `answering_a_suggestion_needs_the_users_answer`, `answering_apart_keeps_the_pair_from_coming_back`, `answering_merge_merges_and_is_undone` y `answering_without_a_pending_suggestion_is_404`;
  - `locating_a_folder_merges_its_own_project_into_the_named_one` y `a_folder_shared_by_its_project_is_not_located`;
  - `a_folder_without_vcs_moved_is_suggested_and_shown_until_answered`;
  - las tres rutas en la tabla de PL19 y en sus dos recorridos.
- [x] GREEN: `POST /v1/projects/resolve`, `POST /v1/projects/suggestions/answer` y `POST /v1/projects/locate`; los nombres en `move_memories` y en el `PATCH`; `suggestions` en `ContextBody`.
- Commit: `feat(daemon): resolve, answer and locate projects`

### 39. El cliente y el puente (PL34, PL36, PL37; D44, D45)

- [x] RED:
  - `saving_to_a_project_by_name_creates_it`, `a_similar_name_is_answered_with_its_candidates`, `project_and_project_path_are_refused_together`, `searching_the_docs_of_a_project_without_a_folder_says_so` y `answer_suggestion_says_what_it_did` (`server.rs`);
  - `a_suggestion_is_shown_until_answered`, en los dos idiomas (`tools.rs`).
- [x] GREEN: los métodos del cliente (`resolve_project`, `answer_suggestion`, `locate_project`), `suggestions` en `Context`; los argumentos `project`, `new_project` y `to_project`; la herramienta `answer_suggestion`; `suggestion_lines` en `format_recall`.
- Commit: `feat(mcp): name a project by id or name and answer its suggestions`

### 40. `session-start` muestra las sugerencias (PL36)

- [x] RED (`hook.rs`): `session_start_names_the_pending_suggestions`.
- [x] GREEN: la línea de `suggestion_note` en el texto de `session-start`, que nombra hasta dos proyectos; el tope de lo que suma el hook pasa de 1.000 a 1.500 bytes (D45).
- Commit: `feat(hooks): name the pending suggestions of the project at session start`

### 41. El contrato del modelo (PL41)

- [x] `tools_es.rs`: los argumentos nuevos y `answer_suggestion`.
- [x] `protocol.md` y `protocol.es.md`: una sección `## Projects` / `## Proyectos`, fuera de la apertura.
- [x] `CLAWG_BLESS=1` de los dos contratos, y revisar el diff.
- Commit: va en el de la tarea 39, porque el contrato fijado cambia con las herramientas.

### 42. `clawg projects create` y `clawg projects locate` (PL39)

- [x] RED: `projects_create_and_locate_parse` (`cli.rs`); `a_created_project_or_its_candidates_are_printed` (`projects.rs`, en los dos idiomas).
- [x] GREEN: `ProjectsAction::Create` y `ProjectsAction::Locate`, sus métodos del cliente y lo que imprimen, con una sentencia por `match` sobre `Lang`.
- Commit: `feat(cli): add projects create and locate`

### 43. Las specs y la documentación

- [x] `specs/project`: PL32 a PL40, y PL35 en R10.
- [x] `specs/mcp`: el argumento `project`, `to_project`, `new_project` y `answer_suggestion`.
- [x] `specs/hooks`: las sugerencias en `session-start`.
- [x] `.claude/CLAUDE.md`: los proyectos sin carpeta y la sugerencia `folder_name` en «Data profiles»; `projects create|locate` en «Commands»; las tres rutas, los nombres en `move` y `PATCH` y `suggestions` en la API; las herramientas MCP.
- Commit: `docs: record projects without a folder and the agent`

### 44. Verificación

- [x] La suite y los dos clippy de `.claude/CLAUDE.md`, en Linux.
- [x] La suite y los dos clippy, en `target-test` en Windows, con `--locked`.
- [x] El CI en Linux y Windows: el del pull request #95 a `develop`. El de `develop` a `main` lo vuelve a correr antes del release.
- [x] El binario en un home temporal, en Linux y en Windows:
  - un save con `project` crea un proyecto sin carpeta, y otro con un nombre parecido devuelve el candidato;
  - `projects create` y `projects locate` desde la CLI;
  - una carpeta sin VCS movida deja su sugerencia, `recall_context` y `session-start` la muestran, y `answer_suggestion` con `apart` la quita.
- [x] `verify.md`, sección «Entrega 3a».
- [x] No queda otro directorio de build que `target` y `target-test`.

## Entrega 3b. Los subproyectos de un monorepo (V30)

Requisitos en la [spec](spec.md#entrega-3b-los-subproyectos-de-un-monorepo) (PL43 a PL52) y decisiones en el [diseño](design.md#entrega-3b-los-subproyectos-de-un-monorepo) (D50 a D59). Antes de la tarea 45: `cargo clean --workspace --target-dir target-test`. La versión sigue en 0.64.0, que todavía no se publicó.

### 45. V30: las declaraciones (PL43; D51)

- [x] RED:
  - `a_database_from_the_previous_version_opens_with_no_subfolder_declared` (`db.rs`, sembrado con `seed_on_disk(dir, Some(29))` y el escenario de PL43);
  - la fila 30 en `published_migration_checksums_are_pinned` (`migrations.rs`);
  - los conteos de migraciones aplicadas de `db.rs` pasan de 29 a 30: `after.0` del test que siembra hasta V8, `Expected exactly 30 applied migrations` y `migration_count`.
- [x] GREEN:
  - `crates/clawg-core/migrations/V30__project_subfolder.sql`: la tabla, su clave primaria `(parent_id, subpath)` y el índice por `project_id`;
  - tocar `migrations.rs` en el mismo commit, para que se recompile.
- Commit: `feat(core): add the subfolders a repository declares as other projects`

### 46. El subcamino de un camino crudo (PL44; D53)

- [x] RED:
  - la pura `the_subpath_is_measured_from_the_top_of_the_checkout` (`identity.rs`): un camino debajo de la cima, la cima misma (vacío), un camino fuera de ella y, en Unix, un componente que no es Unicode válido;
  - `a_path_in_a_worktree_has_its_subpath_from_the_worktree` (con un `git worktree add` real) y `a_path_that_does_not_lead_to_its_root_has_no_subpath` (dos repositorios), en `crates/clawg-core/tests/project_root.rs`.
- [x] GREEN:
  - `subpath_of`, pura;
  - `Ground::subpath(root, path)`: en `Disk`, la cima que encuentra `read_marker` subiendo desde el camino canónico, y su raíz por `root_from_marker`; en `ReadAhead` y en `FakeGround`, lo que tienen;
  - `ProjectIdentity::path`, el camino canónico desde el que sube `resolve_project_root`, y los literales de los tests de `tools.rs` y `server.rs`.
- Commit: `feat(core): measure a path from the top of its checkout`

### 47. La declaración más profunda decide, en el store (PL45; D54, D55)

- [x] RED:
  - la pura `the_deepest_declared_subfolder_wins` (`crates/clawg-core/src/memory/subfolder.rs`): por componentes enteros, la más profunda, la de P antes que la de una lápida y la más vieja entre lápidas;
  - sobre `MemoryStore::new_for_test()` con el `Ground` falso y declaraciones escritas a mano (`project.rs`):
    - `a_declared_subfolder_takes_what_is_saved_inside_it`;
    - `a_declaration_follows_a_moved_checkout`;
    - `a_declaration_is_read_through_merges`;
    - `resolving_a_subfolder_joins_nothing`: un id viejo y el id de la carpeta se unen en P, y `root_id` es el de la raíz.
- [x] GREEN:
  - el módulo `subfolder.rs`, registrado en `memory/mod.rs`, con `deepest` y `declared_in(c, project, subpath)`, que lee las de P y de sus lápidas y sigue `canonical`;
  - `Registration::subpath`, que el daemon lee en la tarea 50, y `RegisterOutcome::subproject`, leído en la pasada que escribe de `register_project`, después de `write_in`.
- Commit: `feat(core): give a path in a declared subfolder the project it leads to`

### 48. Declarar, cambiar y quitar en el store (PL46; D56, D57)

- [x] RED (`subfolder.rs`, sobre `MemoryStore::new_for_test()`): `declaring_a_subfolder_writes_one_row_and_moves_nothing`, `naming_the_repository_project_removes_the_declaration`, `declaring_again_merges_a_project_only_that_subfolder_led_to`, `a_project_reached_otherwise_is_not_merged_by_a_declaration` y `a_declaration_into_a_merged_or_deleted_project_is_refused`.
- [x] GREEN:
  - `declare_subfolder(store, workspaces, backups, parent, subpath, target)`, que devuelve `Declared` (`repository_id`, `subpath`, `project_id`, `already`, `removed`, `merge`);
  - `SubfolderError` con `thiserror`: `Merged`, `Deleted`, `Shared` y la unión;
  - la unión de Q en X por `merge_core`.
- Commit: `feat(core): declare a subfolder of a repository as another project`

### 49. Lo que se ve, la unión y el borrado (PL49, PL50; D58, D59)

- [x] RED:
  - `a_declaration_is_listed_on_both_sides_with_its_paths` y `a_subproject_resolves_to_the_folders_of_its_repository` (`registry.rs`);
  - `merging_and_undoing_leave_the_declarations_as_they_were` (`merge.rs`);
  - `a_delete_report_names_its_subfolders_and_a_declaration_changes_the_code` y la pura `the_code_of_a_project_without_subfolders_does_not_change` (`project_delete.rs`).
- [x] GREEN:
  - `ListedSubfolder` y `ListedProject::subfolders`, con `#[serde(default)]`: leídas en `stored_in`, y sus caminos mirados en el hilo bloqueante de `list_projects`;
  - `folders_of_in`, con las carpetas de los repositorios de un proyecto sin carpetas propias;
  - las declaraciones en `Set`, su línea en `code_of` y `subfolders` en `DeleteReport`.
- Commit: `feat(core): list, resolve and report the subfolders of a project`

### 50. El daemon (PL44 a PL46, PL49)

- [x] RED (`tests/api.rs`):
  - `a_registration_without_a_path_or_with_one_elsewhere_answers_as_before`;
  - `a_declared_subfolder_takes_what_is_saved_inside_it_in_every_clone`, el test RED de la propuesta, con repositorios reales: un monorepo con un `origin` `https://…` puesto con `git remote add` (un remoto `file://` o un camino no tiene clave, R14) y un clone suyo en otro camino, con el mismo `origin`;
  - `a_subfolder_is_declared_through_the_daemon_and_refused_at_a_root`;
  - el brazo de `subfolder` en `every_route_that_takes_a_project_follows_a_merge` y en `every_route_that_takes_a_project_treats_a_deleted_one_by_its_rule`. La tabla de PL19 no cambia: no hay ruta nueva.
- [x] GREEN:
  - `RegisterRequest::path`, leído con el checkout en el mismo `spawn_blocking` de `register_root`, y el `project_id` del subproyecto en la respuesta;
  - `LocateRequest::subfolder`, los campos nuevos de `LocatedBody` y el mapeo de `SubfolderError`;
  - `subfolders` en `GET /v1/projects`.
- Commit: `feat(daemon): resolve and declare the subfolders of a repository`

### 51. Los clientes mandan el camino crudo (PL47)

- [x] RED:
  - `session_start_in_a_declared_subfolder_reads_its_project` (`hook.rs`);
  - `a_bridge_in_a_declared_subfolder_saves_into_its_project`, `a_project_path_in_a_declared_subfolder_goes_to_its_project` y `a_bridge_registers_each_path_of_a_root_once` (`server.rs`);
  - `moving_to_a_declared_subfolder_moves_into_its_project` (`projects.rs`, contra un daemon real);
  - `the_project_of_a_path_reads_the_declarations_without_writing` y `a_store_from_before_subfolders_reads_the_project_of_the_root` (`export.rs`, este último sembrado en V29).
- [x] GREEN:
  - `DaemonClient::register_project` con `path`, y `DaemonClient::locate_project` con `subfolder`, que piden los tests de esta tarea;
  - `project_of` del hook;
  - en el puente, `ids_for` con la clave (raíz, camino), y `save_project` con la misma;
  - `move_memories` (`projects.rs`);
  - `project_id_in` y `project_id_of` con el camino crudo, la tabla temporal para un store anterior a V30, y sus llamadores en `memories.rs` y `main.rs`.
- Commit: `feat(cli): send the path a client works in along with its root`

### 52. `clawg projects locate --subfolder`, la lista y el informe de un borrado (PL48 a PL50)

- [x] RED:
  - `projects_locate_takes_subfolder` (`cli.rs`);
  - `a_declared_subfolder_is_printed_with_what_stayed_behind`, `a_listing_names_the_subfolders` y `a_delete_report_names_its_subfolders`, en español y en inglés (`projects.rs`).
- [x] GREEN: `--subfolder` en `ProjectsAction::Locate`, `render_located` (con `render_declared`), las líneas de `render_list` y de `render_delete`, con una sentencia por `match` sobre `Lang`; `previous` en `Declared` y en la respuesta, el proyecto al que llevaba una declaración quitada, que nombra la salida.
- Commit: `feat(cli): declare a subfolder with projects locate --subfolder`

### 53. Los hallazgos de `doctor` (PL51; D58)

- [x] RED: `a_declaration_whose_projects_are_gone_is_named` (`checks.rs`); la lectura del hecho nuevo en un store en V29 y en V30 (`inspect.rs`).
- [x] GREEN:
  - las declaraciones en `inspect.rs`, leídas sólo donde está la tabla (`Has`);
  - `project_subfolders` en `checks.rs`, con sus mensajes en los dos idiomas;
  - su llamada en `doctor/mod.rs`, y su lugar en `every_finding`.
- Commit: `feat(doctor): name a subfolder declared for a project that is not there`

### 54. Las specs y la documentación

- [x] `specs/project`: PL43 a PL51; R1 (una subcarpeta declarada no es una raíz), R3 (el camino crudo), R4 (`path`) y R6 (la clave raíz y camino).
- [x] `specs/storage`: V30 y su vuelta atrás.
- [x] `specs/doctor`: `project_subfolders`.
- [x] `specs/hooks`: el `cwd` como `path`.
- [x] `specs/mcp`: «Registro del proyecto», con el camino crudo y su clave.
- [x] `specs/portability`: `export --project` con el camino crudo, y el import sin cambios.
- [x] `.claude/CLAUDE.md`:
  - las declaraciones en el párrafo del proyecto de «Data profiles»;
  - `projects locate --subfolder`, y lo que imprimen `projects list` y `projects delete --with-memories`, en «Commands»;
  - `path` en `POST /v1/projects`, `subfolder` en `POST /v1/projects/locate`, y `subfolders` en `GET /v1/projects` y en el informe de un borrado, en la API;
  - `project_subfolders` en `clawg doctor`.
- [x] El contrato del modelo no cambia: `the_agent_contract_matches_its_file` y su variante en español pasan sin volver a bendecir.
- Commit: `docs: record the subfolders of a monorepo`

### 55. Verificación

- [x] La suite y los dos clippy de `.claude/CLAUDE.md`, con `--locked`, en Linux.
- [x] Lo mismo en `target-test` en Windows, con `--locked`, y el binario en un home temporal también en Windows.
- [x] El binario en un home temporal, con un monorepo git real con un `origin` `https://…` y un clone del mismo remoto en otro camino:
  - `projects create web` y `projects locate web <monorepo>/apps/web --subfolder`, cuya salida nombra lo que quedó en el monorepo;
  - `session-start` desde `apps/web/src` lee `web`, un save desde ahí queda en `web`, y uno desde la raíz queda en el monorepo;
  - el clone llega a los dos, y un worktree del monorepo llega a `web` desde su `apps/web`;
  - `projects list` muestra la declaración en los dos lados;
  - `projects locate <monorepo> <monorepo>/apps/web --subfolder` la quita, y lo guardado en `web` sigue allí;
  - `doctor` no marca nada nuevo.
- [x] El CI en Linux y Windows: el del pull request #96 a `develop`. El de `develop` a `main` lo vuelve a correr antes del release.
- [x] `verify.md`, sección «Entrega 3b».
- [x] No queda otro directorio de build que `target` y `target-test`.

## Entrega 4. Exportar e importar (V31)

Requisitos en la [spec](spec.md#entrega-4-exportar-e-importar) (PL53 a PL65) y decisiones en el [diseño](design.md#entrega-4-exportar-e-importar) (D60 a D70). Depende de la 3b: V30 y el camino crudo de `project_id_in` (tarea 51). Antes de la tarea 56: `cargo clean --workspace --target-dir target-test`. La versión sigue en 0.64.0, que todavía no se publicó.

### 56. V31: el diario del registro y el índice por fecha (PL53; D69)

- [x] RED:
  - `a_database_from_the_previous_version_opens_with_its_imports_as_they_were` (`db.rs`, sembrado con `seed_on_disk(dir, Some(30))` y el escenario de PL53: un import anotado en V30, que se deshace después de abrir);
  - la fila 31 en `published_migration_checksums_are_pinned` (`migrations.rs`);
  - los conteos de migraciones aplicadas de `db.rs` pasan de 30 a 31: `after.0` del test que siembra hasta V8, `Expected exactly 31 applied migrations` y `migration_count`.
- [x] GREEN:
  - `crates/clawg-core/migrations/V31__import_registry.sql`: `memory_import_registry` sin `CHECK`, su índice por `import_id`, e `idx_memory_created_at` sobre `memory (created_at)`;
  - tocar `migrations.rs` en el mismo commit, para que se recompile.
- Commit: `feat(core): add the journal of what an import registers and an index by date`

### 57. Una línea `project` por proyecto (PL54, PL55; D60, D61)

- [x] RED:
  - en `format.rs`: `the_pinned_example_reads_and_writes_back_unchanged`, con una segunda línea `project` que lleva todos los campos nuevos; `a_project_line_carries_its_name_locators_and_subfolders`; `a_project_line_without_a_folder_is_read`;
  - en `export.rs`: `an_export_writes_one_line_per_project_with_its_locators` (un clone, un proyecto sin carpeta y una lápida que lleva a uno exportado), `an_export_of_a_subproject_carries_its_repository_and_declaration`, `a_deleted_project_exports_without_its_mark` y `a_store_from_before_the_registry_exports_its_folders_as_before` (sembrado en V26). `export_reads_the_project_id_from_the_store` pasa a esperar también el nombre y el remoto de la línea en un store en V27 o después; en V26, la línea de antes.
- [x] GREEN:
  - `ProjectLine` con `root_path` opcional, y con `name`, `folders` (`FolderLine`), `remotes`, `merged_into` y `subfolders` (`SubfolderLine`), cada uno con `#[serde(default, skip_serializing_if = …)]`;
  - `read`, en `export.rs`: una línea por proyecto, leída con lo que tiene el store (`project` y `project_path` desde V27, `project_remote` desde V24, `merged_into` desde V28, `project_subfolder` desde V30), y las de un store anterior a V27 como hoy;
  - `prepare` deja con su id las filas de una línea sin `root_path`, como las de un proyecto sin línea, y `translate.rs` compila con `root_path` opcional, sin cambiar lo que hace.
- Commit: `feat(core): export one line per project with its name, locators and subfolders`

### 58. Adónde va cada proyecto del archivo (PL55 a PL57; D62, D63)

- [x] RED:
  - las puras `lines_of_one_id_are_one_project` y `a_project_of_the_file_goes_by_map_remote_id_name_then_path` (`crates/clawg-core/src/memory/portability/projects.rs`): cada paso, uno que no decide porque lo contradicen, el nombre con huellas de un solo lado, una línea sin filas que no crea y una lápida que sólo prueba el id;
  - en `import.rs`, sobre `MemoryStore::new_for_test()` con el `Ground` falso:
    - `an_export_from_before_a_merge_lands_in_its_target`, con remoto y sin él;
    - `an_export_from_another_machine_lands_in_the_project_of_its_remote`;
    - `a_project_without_a_folder_goes_and_comes_back_by_name`, con tres stores;
    - `a_project_contradicted_here_is_another_project`;
    - `an_export_from_before_names_and_locators_imports_as_before`, con un archivo literal de antes;
    - `a_row_without_a_line_creates_its_project_as_the_import`;
    - `a_new_project_keeps_the_id_and_name_of_its_line` y `an_id_taken_here_gives_the_new_project_another`;
    - `a_folder_of_another_machine_records_the_fingerprints_of_its_line`;
    - `a_checkout_without_origin_registered_after_an_import_joins_its_project`, que registra después con `register_project`;
  - los casos nuevos de `the_planner_decides_as_the_sql_pass`: un proyecto que va por su remoto y uno que va por su nombre.
- [x] GREEN:
  - el módulo `projects.rs`, registrado en `portability/mod.rs`: `FileProject` (las líneas de un id), `Registry` y `registry_in(c)` (una lectura de `project`, `project_path`, `project_remote` y `project_subfolder`), y la pura `place`, que devuelve el proyecto y el paso que lo decidió (`By`);
  - en `plan`, `registry_in` en la misma llamada que `plan_ids_in`; se planean las carpetas de cada proyecto del archivo, con `--map-root`;
  - `prepare` mapea cada id del archivo con `place`, y `ProjectHere` lleva el proyecto, las huellas del archivo y si la carpeta existe acá (`recorded_checkout_present(camino, None)`);
  - `record_projects`: el proyecto nuevo con el id y el nombre del archivo, o `new_project_id`; la carpeta de otra máquina con las huellas del archivo; la que existe acá, como un registro, salvo contradicha; los remotos del archivo que nadie ata; y el proyecto de una fila sin línea. Cada proyecto que crea, anotado como `project` en `memory_import_registry`;
  - `projects` en `ImportReport`.
- Commit: `feat(core): import each project by its remote, its id and its name before its path`

### 59. Las sugerencias de un import (PL58; D64)

- [x] RED (`import.rs`): `an_import_leaves_the_suggestions_a_registration_would`, `a_project_named_like_one_here_is_suggested`, `a_merge_at_the_source_is_suggested_here` y `a_dry_run_reports_the_suggestions_and_writes_none`. `an_import_records_its_folders_as_a_registration_would` pasa a esperar la sugerencia `remote_changed` de su segunda raíz, en lugar de ninguna.
- [x] GREEN:
  - `RootPlan::suggestions`: `plan_ids_in` calcula las de `plan_in` (`root_commit`, `remote_changed` y `folder_name`) con lo que lee `ReadAhead`, incluidos los checkouts grabados de los proyectos con ese nombre;
  - `place` devuelve las de `similar_name` y `merged_at_source`;
  - `record_projects` las inserta (`INSERT OR IGNORE`) y anota las que insertó; `suggestions` en `ImportReport`.
- Commit: `feat(core): leave the suggestions an import finds, and journal them`

### 60. Un proyecto borrado (PL59; D66)

- [x] RED (`import.rs`): `a_row_imported_deleted_leaves_its_deleted_project_deleted` y `a_live_row_imported_into_a_deleted_project_brings_it_back_and_says_so`.
- [x] GREEN:
  - en `SqlSeat::insert`, antes de una fila borrada, el `deleted_at` y el `reviewed` de su proyecto, y los mismos otra vez después de marcarla;
  - la anotación `revived` cuando una fila viva hizo volver a su proyecto, y la marca en `projects` del reporte.
- Commit: `fix(core): keep a deleted project deleted when an import brings one of its rows deleted`

### 61. Las declaraciones viajan (PL60; D67)

- [x] RED (`import.rs`): `the_subfolders_of_a_repository_travel_with_it`, que después registra el clone con el camino crudo, y `a_declaration_here_is_kept_over_the_file`.
- [x] GREEN: después de `place`, las declaraciones de cada proyecto del archivo con sus proyectos de acá; el subproyecto sin filas al que lleva una declaración; la escritura en `record_projects`, anotada como `subfolder`; `subfolders` en `ImportReport`.
- Commit: `feat(core): carry the subfolders a repository declares through export and import`

### 62. Importar otra vez después de un movimiento (PL62; D65)

- [x] RED (`import.rs`): `an_export_imported_again_after_a_move_adds_nothing` y `two_rows_alike_from_two_origins_stay_two`; la fila de `PRESENT_ELSEWHERE_SQL` en `the_import_lookups_seek_an_index`; el caso de una fila movida en `the_planner_decides_as_the_sql_pass`.
- [x] GREEN: `PRESENT_ELSEWHERE_SQL`, por `idx_memory_created_at`; `Seat::present_elsewhere`, en `SqlSeat` y en `Planned`, que carga esas filas por fecha en `load`; `place` la pregunta para una fila de un export de clawg que no está en su proyecto; `Placement::Elsewhere` y `present_elsewhere` en el reporte.
- Commit: `fix(core): find a moved row wherever it is when an export is imported again`

### 63. Deshacer lo que el import agregó al registro (PL63; D70)

- [x] RED (`undo.rs`): `undoing_an_import_takes_back_what_it_added_to_the_registry`, `what_an_import_added_to_the_registry_stays_once_used` y `undoing_an_import_deletes_again_the_project_it_brought_back`. `an_undo_twice_changes_nothing_the_second_time` y `a_dry_run_undo_reports_without_writing` siguen pasando.
- [x] GREEN: la carga de `memory_import_registry` y del estado de lo que nombra; la decisión pura de qué se retira y qué queda, con su motivo; la escritura en la última transacción, en el orden de D70; `registry` en `UndoReport`.
- Commit: `feat(core): take back what an import added to the registry of projects`

### 64. El daemon (PL56, PL61; D68)

- [x] RED:
  - en `tests/api.rs`:
    - `an_export_of_another_store_imports_by_remote_and_by_name`;
    - `a_project_map_to_a_deleted_ambiguous_or_unknown_project_is_refused` y `a_map_given_as_a_root_is_read_as_before`;
    - el brazo de `/import` (`body.project_map[].project`) en `every_route_says_which_project_it_takes`, `every_route_that_takes_a_project_follows_a_merge` y `every_route_that_takes_a_project_treats_a_deleted_one_by_its_rule`;
  - en `import.rs`: `a_project_map_to_a_project_places_its_notes_there` y `a_project_map_path_in_a_declared_subfolder_goes_to_its_project`.
- [x] GREEN:
  - `MapTarget` (un texto, `{"root", "path"}` o `{"project"}`) en `ImportRequest::map_roots` y `project_map`, y su paso a `ImportOptions`;
  - la referencia resuelta con `find_project` antes de leer el archivo, con sus 404 y 409, y el borrado comprobado otra vez en la transacción que escribe;
  - `MappedProject::root` opcional; `plan_ids_in` con el camino crudo de cada raíz.
- Commit: `feat(daemon): map another tool's project to a project and import raw paths`

### 65. `clawg import` y `clawg export` (PL61, PL64)

- [x] RED:
  - `import_takes_a_project_map_to_a_project` (`cli.rs`);
  - `the_maps_are_sent_with_their_raw_paths`, y `an_import_report_names_where_each_project_went` y `render_undo_names_what_it_took_back_from_the_registry`, en español y en inglés (`crates/clawg-cli/src/import.rs`);
  - `an_export_counts_its_projects` (`crates/clawg-cli/src/export.rs`);
  - `the_reasons_of_an_import_are_worded`, en los dos idiomas (`projects.rs` y `tools.rs`).
- [x] GREEN:
  - `ProjectMap` con un destino (un camino o un proyecto) y su parseo por el último `=project:`;
  - `DaemonClient::import`, que manda los objetos;
  - `render` y `render_undo` con las líneas nuevas, y la salida de un import que termina con `clawg projects list` cuando creó proyectos o dejó sugerencias;
  - `similar_name` y `merged_at_source` en `suggestion_reason`, `reason` y `candidate_reason`, y `folder_name` en este último, que hoy lo deja como identificador;
  - el conteo de proyectos en la salida de `clawg export`;
  - todo con una sentencia por `match` sobre `Lang`.
- Commit: `feat(cli): map a project of another tool to a project and say where each imported project went`

### 66. Las specs y la documentación

- [x] `specs/portability`: R1 (una línea `project` por proyecto, con sus campos), R4 (el orden de PL56, lo que se graba y las sugerencias), R5 (la fila presente en cualquier proyecto), IU1 e IU2 (`memory_import_registry` y lo que retira el `--undo`), IL1 (`PRESENT_ELSEWHERE_SQL`), y HI7 y HI16 (`--project-map NOMBRE=project:REF` y el camino crudo).
- [x] `specs/storage`: V31 y su vuelta atrás.
- [x] `specs/project`: PL53 a PL64, y lo que cambia en PL6, PL7 y PL47.
- [x] `.claude/CLAUDE.md`:
  - en el párrafo del proyecto de «Data profiles»: cómo decide el import, sus sugerencias y lo que hace con un proyecto borrado;
  - en «Commands»: `clawg export` (una línea por proyecto), `clawg import` (el orden, `--project-map NOMBRE=project:REF`, los caminos crudos, la fila presente en cualquier proyecto y lo que dice el reporte) y `clawg import --undo` (lo que retira del registro);
  - en la API: el cuerpo de `POST /v1/import` y el reporte del `--undo`.
- [x] El contrato del modelo no cambia: `the_agent_contract_matches_its_file` y su variante en español pasan sin volver a bendecir.
- Commit: `docs: record how projects travel through export and import`

### 67. Verificación

- [x] La suite y los dos clippy de `.claude/CLAUDE.md`, con `--locked`, en Linux.
- [x] Lo mismo en `target-test` en Windows, con `--locked`, y el binario en dos homes temporales también en Windows.
- [x] El binario en dos homes temporales, A y B, con un repositorio git real con un `origin` `https://…`, clonado en otro camino en cada home:
  - en A: un proyecto sin carpeta creado con `projects create`, memorias en él y en el repositorio, y un monorepo con su declaración; después, `clawg export`;
  - en B: `clawg import`. Las memorias del repositorio quedan en el proyecto del clone de B, las del proyecto sin carpeta en uno con su nombre, y la declaración lleva al subproyecto;
  - otra vez `clawg import` en B, después de un `projects move`: no agrega nada;
  - `clawg import --undo N` deja `projects list` como estaba;
  - un export de engram con `--project-map NOMBRE=project:ideas`, y el 409 con un proyecto borrado;
  - `doctor` no marca nada nuevo.
- [x] El CI en Linux y Windows: el del pull request #97 a `develop`, sobre todo el cambio. El de `develop` a `main` lo vuelve a correr antes del release.
- [x] `verify.md`, sección «Entrega 4».
- [x] No queda otro directorio de build que `target` y `target-test`.

## Archive

### 68. Archive

- [x] Los requisitos ya estaban en las specs, consolidados con cada entrega. El «Origen» de `specs/project`, `specs/storage`, `specs/portability`, `specs/doctor`, `specs/mcp`, `specs/hooks` y `specs/secrets` apunta al archivo y dice 0.65.0.
- [x] `git mv` a `changes/archive/2026-10-27-project-locators`, con `state.yaml` archivado y los enlaces relativos que rompió el traslado ajustados, también los del archivo de 89. Se cambiaron la fila 93 y el orden de ejecución del `README.md` de openspec, y el backlog suma los dos hallazgos de Windows.
- [x] La versión no sube: 0.65.0 todavía no se publicó y ya trae el cambio ([verify](verify.md#al-archivar-2026-10-27)).
- Commit: `docs(openspec): archive project locators and fold them into the specs`
