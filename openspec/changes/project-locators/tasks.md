# project-locators — Tareas

Strict TDD: en cada tarea, el test va primero y falla, por la aserción o porque no compila. Cada tarea es un commit convencional que compila y pasa la suite (`cargo test --workspace --target-dir target-test` en Windows) y clippy (`cargo clippy --workspace --all-targets --target-dir target-test -- -A clippy::all -D warnings`). Antes de empezar una entrega: `cargo clean --workspace --target-dir target-test`. Las tareas de las entregas 2 a 4 se escriben cuando les toque.

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
- [ ] El CI del pull request en Linux y Windows.
- [ ] `verify.md`, sección «Entrega 1».
- [ ] No queda otro directorio de build que `target` y `target-test`.
