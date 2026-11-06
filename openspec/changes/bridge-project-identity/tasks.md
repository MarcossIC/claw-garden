# bridge-project-identity — Tareas

Strict TDD: en cada tarea con conducta nueva, el test va primero y falla, por la aserción o porque lo que llama no existe todavía, y el código que lo hace pasar va en el mismo commit. Los tests de «Cómo se mide» de la [propuesta](proposal.md#cómo-se-mide-el-éxito) se escriben antes que cualquier código. Cada tarea es un commit convencional y se comprueba con los tests de los archivos que cambia. La suite y los dos clippy corren una vez, en el último commit. Los commits se arman con rutas explícitas, sin referencias a planes, fases, cambios ni números de tarea.

Los requisitos están en la [spec](spec.md); las decisiones, en la [propuesta](proposal.md#decisiones).

## 0. Antes del código

- [x] El problema de la exploración, verificado otra vez sobre `27c4a88e`.
- [x] `proposal.md`, `spec.md`, este archivo y `state.yaml`.
- Commit: `docs(openspec): propose bridge-project-identity`

## 1. Los tests de «Cómo se mide», en rojo

- [ ] Un cliente MCP de prueba en `crates/clawg-cli/src/mcp/mod.rs`: habla por el stdio en memoria del puente, declara o no la capacidad `roots`, contesta `roots/list` con las raíces del momento y falla si se la piden sin haberla declarado; el puente arranca contra un daemon en proceso que ya publicó sus archivos, con el directorio de arranque y el personal elegidos por el test.
- [ ] Tests en rojo: `a_save_without_a_project_is_refused_with_the_roots_as_options`, `a_save_without_a_project_is_refused_without_roots_too`, `a_search_with_one_root_reads_its_project`, `a_search_with_several_roots_reads_all_of_them_and_names_each_project`, `a_search_without_roots_reads_the_start_directory`, `the_home_directory_is_never_a_project`, `a_root_as_a_path_and_as_a_file_uri_is_the_same_project` y `a_change_of_roots_between_two_calls_is_seen_in_the_second` (BP1 a BP7).
- Van en los commits de las tareas 2 a 4, con el código que los hace pasar.

## 2. Buscar en un grupo de proyectos (BP6)

- [ ] Tests en rojo: `a_group_search_takes_each_row_once` (`crates/clawg-core/src/memory/search.rs`), `a_search_takes_a_group_of_projects` (`crates/clawg-cli/tests/api.rs`) y `a_group_search_names_its_first_project_for_an_older_daemon` (`crates/clawg-cli/src/client.rs`).
- [ ] `search_projects` en el core; `project_ids` en `POST /v1/search`; `SearchArgs.project_ids` en el cliente.
- Commit: `feat(search): search a group of projects in one request`

## 3. Las raíces del cliente y dónde lee una llamada (BP1 a BP4, BP6 a BP9)

- [ ] Tests en rojo: `a_root_is_a_folder_as_a_uri_or_as_a_path` (`crates/clawg-cli/src/mcp/roots.rs`), `a_group_hit_names_its_project_and_its_key` (`tools.rs`), `what_works_on_one_project_asks_for_one_when_several_are_open` y `a_declared_root_is_indexed_like_the_start_directory` (`server.rs`), y los de lectura de la tarea 1.
- [ ] `roots.rs`; el directorio de arranque y el personal como argumentos de `run_with`; `roots/list` en cada llamada; las carpetas y los proyectos de lectura; la búsqueda en grupo con `project=`; la negativa con varias de las otras herramientas; `index_docs` de una raíz.
- Commit: `feat(mcp): read in the projects the client has open`

## 4. Un save nombra su proyecto (BP5, BP10, BP11)

- [ ] Tests en rojo: `a_save_names_its_project_or_is_refused_before_the_daemon` (`server.rs`), los de save de la tarea 1, `the_protocol_says_where_a_call_without_a_project_goes` (`protocol.rs`), y `a_hit_without_a_project_shows_how_a_save_revises_it` y `format_save_shows_the_key_only_where_a_save_revises_it` (`tools.rs`) con la regla nueva.
- [ ] La negativa del save; la clave de una fila sin proyecto; los tests que guardaban sin proyecto nombran el suyo (`server.rs`, `crates/clawg-cli/tests/binary.rs`).
- [ ] `protocol.md` y `protocol.es.md`, la apertura y «Projects»; las descripciones de `project_path` y `project`, en inglés y en `tools_es.rs`.
- [ ] `CLAWG_BLESS=1` para los dos contratos, el diff revisado, y los tests de nuevo.
- Commit: `feat(mcp): refuse a save that does not name its project`

## 5. Docs

- [ ] `docs/reference/daemon.md`: `project_ids` de `POST /v1/search`, las raíces y las carpetas de lectura del puente, el save que nombra su proyecto, `project=` en un hit de un grupo. `docs/reference/data-profiles.md`: el camino que manda el puente.
- Commit: `docs(reference): say where the bridge reads and that a save names its project`

## 6. Verificación

- [ ] La suite, `cargo clippy --workspace --all-targets -- -D warnings` y `cargo clippy -p clawg-cli -- -D warnings`, sobre el último commit de código ([verify](verify.md)).
- [ ] El CI del pull request en Linux y Windows; lo que marque se arregla.
- [ ] `verify.md`, con el save desde Cursor en un workspace de dos repositorios como pendiente en local; `state.yaml`; la línea del cambio en `ROADMAP.md` y su fila del mapa de openspec.
- Commit: `docs(openspec): record bridge-project-identity applied`

## 7. Archivo (fuera de este cambio)

- [ ] Llevar BP1 a BP8, BP10 y BP11 a `specs/mcp`, BP9 a `specs/project-docs` y `specs/project`, y cambiar R3 y PL47 de `specs/project`; mover la carpeta a `changes/archive/`; subir el minor.
