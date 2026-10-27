# project-registry — Tareas

Strict TDD: en cada tarea el test va primero y falla, por la aserción o porque no compila. Cada tarea es un commit convencional que compila y pasa la suite. Las tareas de las entregas 1 a 4 pasaron, sin escribir, a [project-locators](../2026-10-27-project-locators/proposal.md) (93).

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
- 0.62.1 no se publicó: al archivar ram-footprint la versión pasó a 0.63.0, que es la que trae la entrega 0.

### 5. Verificación

- [x] Suite y clippy de `.claude/CLAUDE.md` en Linux, y el binario en un home temporal.
- [x] El CI del pull request #93 en verde en Linux y Windows.
- [x] `verify.md`.

### 6. Archive

- [x] El «Origen» de `specs/project` y de `specs/doctor` apunta al archivo, y la fila `project_remotes` de `specs/doctor` dice 0.63.0. `git mv` a `changes/archive/2026-10-25-project-registry`, con `state.yaml` archivado. El cambio se partió: las entregas 1 a 4 pasaron a `changes/project-locators` (93). Se cambiaron las filas 89 y 93 y el orden de ejecución del `README.md` de openspec. La versión no sube ([verify](verify.md#al-archivar-2026-10-25)).
