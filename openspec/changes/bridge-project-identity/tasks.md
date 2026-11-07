# bridge-project-identity — Tareas

Strict TDD: en cada tarea con conducta nueva, el test va primero y falla, por la aserción o porque lo que llama no existe todavía, y el código que lo hace pasar va en el mismo commit. Lo que se mueve sin conducta nueva lo fijan los tests que ya hay, que pasan antes y después sin cambiar lo que esperan. Cada tarea es un commit convencional, comprobado con los tests de los archivos que cambia; la suite y los dos clippy corren una vez, en el último commit. Los commits se arman con rutas explícitas, sin referencias a planes, fases, cambios ni números de tarea.

Los requisitos están en la [spec](spec.md); las decisiones, en la [propuesta](proposal.md#decisiones).

Lo que se hizo distinto de lo previsto está en [«Frente a lo previsto»](verify.md#frente-a-lo-previsto) de la verificación.

## 0. Antes del código

- [x] El problema de la exploración, verificado otra vez sobre `27c4a88`.
- [x] `proposal.md`, `spec.md`, este archivo y `state.yaml`.
- Commit: `docs(openspec): propose bridge-project-identity`

## 1. Las raíces y las carpetas abiertas (BI2, BI3)

- [x] Tests en rojo: `a_root_is_read_as_a_uri_or_a_path`, `a_root_as_a_path_and_as_a_uri_is_one_project`, `the_open_folders_are_the_roots_else_the_directory` y `the_home_directory_is_never_a_project` (`roots.rs`; no compilan: no hay módulo).
- [x] `crates/clawg-cli/src/mcp/roots.rs`: `folder_of` y `open_folders`, puros.
- Commit: `feat(mcp): read a client's roots as the folders it has open`

## 2. La búsqueda de un grupo en el core (BI6)

- [x] Tests en rojo: `a_group_scope_reads_each_of_its_projects` (`search.rs`) y `counting_a_group_counts_each_of_its_projects` (`language.rs`).
- [x] El predicado de proyecto ata una lista JSON; `SearchScope` guarda la lista, con `with_projects`; cada consulta que usa el predicado ata un proyecto como una lista de uno (`search.rs`, `language.rs`, `dispute.rs`, `crowding.rs`, `prompt_builder.rs`, `eval_api.rs`). Los tests del core con un proyecto pasan sin cambiar lo que esperan.
- Commit: `feat(search): search a group of projects`

## 3. `project_ids` en `POST /v1/search` (BI6)

- [x] Tests en rojo: `a_search_takes_a_group_of_projects` y `a_search_refuses_a_project_and_a_group` (`crates/clawg-cli/tests/api.rs`), `a_search_sends_a_group_only_when_it_has_one` (`client.rs`).
- [x] `SearchRequest.project_ids` y su paso por `live_project`; el conteo del otro idioma del grupo; `SearchArgs.project_ids` en el cliente.
- Commit: `feat(daemon): take a group of projects in a search`

## 4. Las raíces de cada llamada en el puente (BI1, BI3, BI4, BI5, BI7 a BI9)

- [x] Tests en rojo, en `server.rs`: los de BI1 por el `duplex` (`the_roots_are_asked_on_each_call`, `a_client_without_roots_is_not_asked`, `a_roots_error_counts_as_no_roots`), los del save (BI4), los de la búsqueda (BI5), `a_bridge_started_in_the_home_directory_registers_no_project`, `a_one_project_tool_asks_which_folder_when_several_are_open`, `memory_status_counts_every_project_with_several_folders`, `read_memory_shows_the_key_of_a_row_in_any_open_folder` y `a_root_the_client_declares_is_indexed_like_the_directory`; en `tools.rs`, `format_hits_names_the_project_of_each_hit_of_a_group`, `the_key_shows_on_a_row_of_any_open_folder` y `only_an_open_folder_is_indexed`.
- [x] El pedido de `roots/list` con su plazo (`roots.rs`); cada herramienta de BI1 pide las raíces y trabaja sobre un clon con ellas; el save sin proyecto, el grupo de `search_memory`, las negativas de BI7, `memory_status`, `read_memory` y el índice de documentos; el directorio personal fuera de `ClawgMcp::pending`. Los tests que guardaban sin proyecto pasan `project_path` (`save_args`).
- Commit: `feat(mcp): read the folders the client has open and make every save name its project`

## 5. Lo que ve el modelo (BI10)

- [x] Las descripciones de `project_path`, `project` y `search_memory` en inglés y en español (`tools_es.rs`); la apertura y la sección de proyectos de `protocol.md` y `protocol.es.md`.
- [x] `CLAWG_BLESS=1` para los dos contratos, el diff revisado, y los tests de nuevo.
- Commit: `feat(mcp): tell the model that a save names its project`

## 6. Docs

- [x] `docs/reference/daemon.md`: las herramientas MCP sin proyecto, las raíces y `project_ids`. `docs/reference/data-profiles.md`: de dónde sale el proyecto del puente. README: la lista de herramientas MCP y la sección de Cursor.
- Commit: `docs: describe how the bridge picks a project from the client's roots`

## 7. Verificación

- [x] `cargo test --workspace` y `cargo clippy --workspace --all-targets -- -D warnings` en Linux, sobre el último commit.
- [x] `verify.md`, con el save desde Cursor en un workspace con dos repositorios pendiente en local; `state.yaml` y la línea de `openspec/ROADMAP.md`.
- [x] El CI del pull request en Windows, y lo que marque, arreglado: 18 tests de `server.rs` armaban su proyecto con una ruta que en Windows resuelve a otro id; arreglado en `test(mcp): name the bridge's own folder as a project_path resolves it`.
- Commit: `docs(openspec): record bridge-project-identity applied`
