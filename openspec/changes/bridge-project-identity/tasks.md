# bridge-project-identity — Tareas

Strict TDD: en cada tarea con conducta nueva, el test va primero y falla, por la aserción o porque lo que llama no existe todavía, y el código que lo hace pasar va en el mismo commit. Los tests de «Cómo se mide» de la [propuesta](proposal.md#cómo-se-mide-el-éxito) se escriben antes que cualquier código. Cada tarea es un commit convencional y se comprueba con los tests de los archivos que cambia. La suite y los dos clippy corren una vez, en el último commit. Los commits se arman con rutas explícitas, sin referencias a planes, fases, cambios ni números de tarea.

Los requisitos están en la [spec](spec.md); las decisiones, en la [propuesta](proposal.md#decisiones).

## Frente a lo previsto

| Punto | Previsto | Hecho | Por qué |
|---|---|---|---|
| Tareas 3 y 4 | dos commits | uno, `feat(mcp): read in the projects the client has open, and make a save name its own` | Cada mitad sola deja un estado intermedio falso: lecturas desde las raíces mientras un save sigue cayendo en el proyecto del directorio de arranque, que en Cursor es el directorio personal; o saves sin proyecto por defecto mientras la clave de un hit se muestra como si un save común fuera a ninguno |
| BP1, qué herramientas piden las raíces | todas | las que leen o guardan: `save_memory`, `search_memory`, `recall_context`, `read_memory`, `synthesize`, `search_project`, `answer_suggestion` y `memory_status`; la spec lo dice | Las otras no dependen del proyecto de la llamada, y pedir les costaría una ida y vuelta sin uso |
| El 403 de `search_project` (`not_accepted`, `crates/clawg-cli/src/daemon/api.rs`) | no estaba | dice que el daemon lee una carpeta que el cliente tiene abierta o en la que arrancó un puente; la spec lo dice en BP9 | Decía sólo «la raíz en la que arrancó un puente», que con las raíces deja de ser cierto, y es lo que lee el modelo |
| `test_project` (`server.rs`) | hecho a mano con `/proj-test` | `ProjectIdentity::resolve("/proj-test")`, y `save_args` nombra `PROJ_TEST` | En Windows `/proj-test` resuelve a `C:\proj-test`: hecho a mano, un save con ese `project_path` iba a otro proyecto que el del puente. En Linux es el mismo de antes |
| El fixture `repo_with_src` (`server.rs`) | — | sin el `\\?\` de la ruta canónica de Windows | Su legacy id salía de una ruta con el prefijo, que ni un directorio de trabajo ni un `project_path` llevan; con los saves que nombran su carpeta, `a_bridge_in_a_subdirectory_brings_what_was_saved_there_to_the_root` lo dejaba ver en Windows |
| `resolve_project` (`tools.rs`) y sus cuatro tests | — | borrados | Era la regla del proyecto por defecto, que ya no existe; la precedencia (`all_projects`, `project`, `project_path`, carpetas de lectura) vive en `scope_for` |
| Los tests de BP5, BP8 y BP9 en `server.rs`, de BP2 y BP3 en `roots.rs`, y los de BP10 en `tools.rs` | antes del código | escritos con el código o después, y cada uno visto fallar contra una mutación de la línea que lo hace pasar ([verify](verify.md#los-tests-en-rojo)) | Los de «Cómo se mide» se vieron fallar antes de todo el código; estos se sumaron mientras se escribía |

## 0. Antes del código

- [x] El problema de la exploración, verificado otra vez sobre `27c4a88e`.
- [x] `proposal.md`, `spec.md`, este archivo y `state.yaml`.
- Commit: `docs(openspec): propose bridge-project-identity`

## 1. Los tests de «Cómo se mide», en rojo

- [x] Un cliente MCP de prueba en `crates/clawg-cli/src/mcp/mod.rs`: habla por el stdio en memoria del puente, declara o no la capacidad `roots`, contesta `roots/list` con las raíces del momento y falla si se la piden sin haberla declarado; el puente arranca contra un daemon en proceso que ya publicó sus archivos, con el directorio de arranque y el personal elegidos por el test (`run_with` los recibe en `Start`).
- [x] Tests en rojo: `a_save_without_a_project_is_refused_with_the_roots_as_options`, `a_save_without_a_project_is_refused_without_roots_too`, `a_search_with_one_root_reads_its_project`, `a_search_with_several_roots_reads_all_of_them_and_names_each_project`, `a_search_without_roots_reads_the_start_directory`, `the_home_directory_is_never_a_project`, `a_root_as_a_path_and_as_a_file_uri_is_the_same_project` y `a_change_of_roots_between_two_calls_is_seen_in_the_second` (BP1 a BP7), y `a_search_takes_a_group_of_projects` (`crates/clawg-cli/tests/api.rs`, BP6).
- Van en los commits de las tareas 2 y 3, con el código que los hace pasar.

## 2. Buscar en un grupo de proyectos (BP6)

- [x] Tests en rojo: `a_group_search_takes_each_row_once` (`crates/clawg-core/src/memory/search.rs`), `a_search_takes_a_group_of_projects` (`crates/clawg-cli/tests/api.rs`) y `a_group_search_names_its_first_project_for_an_older_daemon` (`crates/clawg-cli/src/client.rs`). `every_route_that_takes_a_project_follows_a_merge` busca también con un grupo.
- [x] `search_projects` en el core; `project_ids` en `POST /v1/search`; `DaemonClient::search_group`, que manda el primero del grupo como `project_id`.
- Commit: `feat(search): search a group of projects in one request`

## 3. Las raíces del cliente, dónde lee una llamada y el save que nombra su proyecto (BP1 a BP11)

- [x] Tests en rojo: los de la tarea 1; `a_root_is_a_folder_as_a_uri_or_as_a_path`, `a_root_that_names_no_folder_here_is_dropped` y `the_home_directory_and_its_root_are_no_folder` (`crates/clawg-cli/src/mcp/roots.rs`); `a_group_hit_names_its_project_and_its_key` y `only_a_folder_the_person_opened_is_indexed` (`tools.rs`), y `a_hit_without_a_project_shows_how_a_save_revises_it` y `format_save_shows_the_key_only_where_a_save_revises_it` con la regla de BP10; `a_save_names_its_project_or_is_refused_before_the_daemon`, `what_works_on_one_project_asks_for_one_when_several_are_open` y `a_declared_root_is_indexed_like_the_start_directory` (`server.rs`); `the_protocol_says_where_a_call_without_a_project_goes` (`protocol.rs`), y la oración nueva en `the_instructions_carry_the_saving_habits` y `the_spanish_instructions_carry_the_saving_habits`.
- [x] `roots.rs`: `Start`, `Folder`, `root_path`, `folders` y `declared` (`roots/list` con `ROOTS_WAIT`).
- [x] `server.rs`: el servidor de cada llamada (`at`, `with_roots`), las carpetas y los proyectos de lectura (`reading_folders`, `reading`), `register`, `scope_for`, la negativa del save (`unplaced`) y la de varias (`one_of`, `several_open`), `index_docs` de una raíz (`indexable_roots`), sin proyecto por defecto.
- [x] `tools.rs`: `format_group_search` con `project=`, `revised_from` sin save común a ninguno, `indexes_docs` sobre varias raíces. `not_accepted` del daemon.
- [x] Los tests que guardaban sin proyecto nombran el suyo (`server.rs`, `crates/clawg-cli/tests/binary.rs`).
- [x] `protocol.md` y `protocol.es.md`, la apertura, «How to write a save» y «Projects»; las descripciones de `project_path` y `project`, en inglés y en `tools_es.rs`.
- [x] `CLAWG_BLESS=1` para los dos contratos, el diff revisado ([verify](verify.md#el-contrato-del-agente)), y los tests de nuevo.
- Commit: `feat(mcp): read in the projects the client has open, and make a save name its own`

## 4. Docs

- [x] `docs/reference/daemon.md`: `project_ids` de `POST /v1/search`, las raíces y las carpetas de lectura del puente, el save que nombra su proyecto, `project=` en un hit de un grupo, la regla de la clave y qué raíces se indexan. `docs/reference/data-profiles.md`: el camino que manda el puente. `docs/reference/commands.md`: qué indexa el daemon sin `clawg projects add`.
- Commit: `docs(reference): say where the bridge reads and that a save names its project`

## 5. Verificación

- [x] La suite, `cargo clippy --workspace --all-targets -- -D warnings` y `cargo clippy -p clawg-cli -- -D warnings`, sobre el último commit de código ([verify](verify.md)).
- [x] El CI del pull request en Linux y Windows; lo que marque se arregla. Pasó en [#134](https://github.com/MarcossIC/claw-garden/pull/134) sin nada que arreglar.
- [x] `verify.md`, con el save desde Cursor en un workspace de dos repositorios como pendiente en local; `state.yaml`; la línea del cambio en `ROADMAP.md` y su fila del mapa de openspec.
- Commit: `docs(openspec): record bridge-project-identity applied`

## 6. Archivo (fuera de este cambio)

- [ ] Llevar BP1 a BP8, BP10 y BP11 a `specs/mcp`, BP9 a `specs/project-docs` y `specs/project`, y cambiar R3 y PL47 de `specs/project`; mover la carpeta a `changes/archive/`; subir el minor.
