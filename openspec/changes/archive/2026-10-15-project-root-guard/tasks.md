# project-root-guard — Tareas

Strict TDD: en cada tarea el test va primero y falla por la aserción o por no compilar. Una tarea, un commit convencional que compila y pasa la suite.

## 1. La marca en `project` (R8, D2)

- [x] Test `a_database_from_the_previous_version_keeps_its_projects_indexed` (`db.rs`): siembra hasta `V12` y falla porque no existe `index_docs`.
- [x] `crates/clawg-core/migrations/V13__project_index_docs.sql`.
- Commit: `feat(core): record whether the daemon may read a project's files`

## 2. El registro guarda y sube la marca (R2, D3)

- [x] Tests `registering_to_index_raises_the_mark_and_never_lowers_it` y `a_collision_leaves_the_other_roots_mark_alone` (`project.rs`).
- [x] `register_project(..., index_docs)` y `RegisterOutcome::index_docs`; los callers de hoy pasan `true` para no cambiar comportamiento todavía.
- Commit: `feat(core): register a project with or without reading its files`

## 3. El índice no lee una raíz no aceptada (R1, D4)

- [x] Test `a_root_that_was_not_accepted_is_not_read` (`project_docs/mod.rs`).
- [x] `root_of` lee la marca; `DocIndexError::NotAccepted`.
- Commit: `feat(core): refuse to index a project root the user did not accept`

## 4. El daemon (R1, R2, R3, R4)

- [x] Tests en `crates/clawg-cli/tests/api.rs`: `a_root_registered_without_its_docs_is_not_indexed`, `searching_the_docs_of_a_root_that_was_not_accepted_is_forbidden`, `registering_a_missing_root_to_index_it_is_a_bad_request`; el puro del texto del 403 en `api.rs`.
- [x] `RegisterRequest::index_docs` (falso si falta), 400 por raíz que no es directorio, refresco sólo si quedó aceptada, 403 en `search_docs`. Los tests que hoy registran por HTTP y esperan el índice mandan `index_docs: true`.
- [x] `DaemonClient::register_project(..., index_docs)`; el puente pasa `true` por ahora.
- Commit: `feat(daemon): index a registered project only once it is accepted`

## 5. El puente pide indexar sólo su raíz (R3, R5, D5)

- [x] Tests `only_the_bridges_own_root_is_indexed` (`tools.rs`), `a_save_elsewhere_registers_the_root_without_reading_it` y `search_project_does_not_read_a_root_the_user_did_not_add` (`server.rs`).
- [x] `indexes_docs`, su uso en `project_for` y el brazo del 403 en `search_project_impl`.
- [x] Descripción de `project_path` en `SearchProjectArgs`; contrato bendecido con `CLAWG_BLESS=1` y diff revisado.
- Commit: `feat(mcp): index only the project the bridge was started in`

## 6. Colisión ruidosa (R6, R7, D5)

- [x] Tests `a_colliding_root_refuses_the_save_every_time` (reemplaza a `a_refused_registration_does_not_block_the_save`), `a_colliding_root_still_searches_all_projects` y `a_registration_the_daemon_cannot_do_does_not_block_the_save` (`server.rs`).
- [x] `project_for` devuelve `Result`; las cinco herramientas lo convierten en error.
- Commit: `feat(mcp): refuse a project whose id already belongs to another root`

## 7. `clawg projects add` (R4, D6)

- [x] Tests `projects_add_takes_a_path_and_a_profile` (`cli.rs`), `adding_a_project_lets_its_docs_be_searched` y `adding_a_path_that_is_not_a_directory_is_refused` (`projects.rs`).
- [x] `Command::Projects`, `crates/clawg-cli/src/projects.rs` y su rama en `main.rs`.
- Commit: `feat(cli): add a project whose files the daemon may read`

## 8. Docs

- [x] `.claude/CLAUDE.md`: el comando, `index_docs` en `POST /v1/projects`, el 403 de la búsqueda de docs y la regla en la sección de profiles o del índice.
- Commit: `docs: describe which project roots the daemon reads`

## 9. Verificación

- [x] `cargo test --workspace` en cada commit, clippy con `-A clippy::all -D warnings`, `verify.md` con el conteo contra la base.
- [x] Pendiente en Windows (máquina de Marcos): la suite y clippy. Sobre `main` en `210c692` fallaba `adding_a_project_lets_its_docs_be_searched`, un error del test con el prefijo `\\?\`; arreglado en `8312e42`, pasan 1.984 y no falla ninguno. `clippy` sale con código 0.

## 10. Archive

- [x] [`specs/project-docs`](../../../specs/project-docs/spec.md) y [`specs/project`](../../../specs/project/spec.md) con los requisitos, la línea **Origen** y los `ruta:línea` revisados contra `main`.
- [x] `git mv` a `changes/archive/2026-10-15-project-root-guard`; `state.yaml` archivado; fila 29 y orden de ejecución del `README.md` de openspec; base de tests de `.claude/CLAUDE.md` y `openspec/config.yaml`; subir el minor.
