# project-registry — Tareas

Strict TDD: en cada tarea el test va primero y falla, por la aserción o porque no compila. Cada tarea es un commit convencional que compila y pasa la suite. Las tareas de las entregas 1 a 4 se escriben cuando les toque.

## Entrega 0. El fundido (versión patch, sin esquema)

### 1. La huella valida la fila de un camino (RG1 a RG4)

- [x] `a_path_settles_on_the_row_its_remote_does_not_contradict` (pura) y, sobre `MemoryStore::new_for_test()`, `a_repository_in_the_path_of_another_gets_a_project_of_its_own`, `a_repository_in_the_path_of_another_takes_none_of_its_rows`, `each_repository_in_one_path_returns_to_its_own_project`, `the_project_of_a_repository_in_another_s_path_is_read_alike_without_writing`, `a_first_remote_keeps_the_project_of_its_root`, `a_caller_that_derives_its_id_is_refused_another_repository_s_project`; `a_new_remote_keeps_the_project_of_its_root` pasa a `a_different_remote_in_a_registered_path_is_another_project` (`project.rs`). En la API, `a_repository_cloned_where_another_was_registers_into_a_project_of_its_own` (`tests/api.rs`). Fallaban cinco de `project.rs` y el de la API.
- [x] `settle`, `path_rows_in`, `settle_root_in` con `Pending` para el lote, `RootId::owns_legacy`; `root_id_in`, `project_id_in`, `plan_ids_in` y `plan_in` por ahí; el handler lee `origin_key` también sin `assigned`.
- Commit: `fix(core): give a repository cloned where another was a project of its own` (`40694b2`). Al probarlo con el binario, el `detail` del 409 nombraba como dueño del id al mismo camino pedido: `fix(daemon): say why a root that derives its own id is refused its own path` (`09feef4`).

### 2. `doctor` nombra los proyectos con más de un remoto (RG5)

- [x] `the_facts_name_the_remotes_of_each_project` (`inspect.rs`), `project_remote_findings` y los dos idiomas en `every_finding` (`doctor/checks.rs`).
- [x] `RemoteRow` y `StoreFacts::remotes`; `checks::project_remotes`; `doctor::run` lee la clave de cada raíz.
- Commit: `feat(doctor): name the projects that hold more than one remote` (`a7d7840`)

### 3. Las specs y la documentación

- [x] `specs/project` (R15, R16, R18 y R20), `specs/doctor` (R7) y `.claude/CLAUDE.md`.
- Commit: `docs: record how a path settles on the row of its repository`

### 4. La versión

- [x] `0.62.0` → `0.62.1` (un parche: la entrega 0 se publica sola) y `cargo update --workspace`.
- Commit: `chore: bump version to 0.62.1`
