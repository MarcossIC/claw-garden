# bridge-project-identity — Verificación

## Suite y clippy

En Windows x86_64 (`x86_64-pc-windows-msvc`), build debug, toolchain 1.99.1, con `--target-dir target-test`, sobre `89a61907`: el último commit, de documentación, con el código de `51410a4f`.

- `cargo test --workspace --no-fail-fast` pasa, sin ningún test fallado. Los ignorados son los `#[ignore]` que ya tenía el repositorio: este cambio no agrega ninguno.
- `cargo clippy --workspace --all-targets -- -D warnings` sale con 0, sin ningún warning, y `cargo clippy -p clawg-cli -- -D warnings`, lo que construye el release, también.
- Los contratos del agente pasan después de bendecirlos con `CLAWG_BLESS=1`, en los dos idiomas ([abajo](#el-contrato-del-agente)).
- El código `cfg(windows)` que toca el cambio (la lectura de una raíz como ruta de Windows, que en realidad es pura y se prueba en los dos sistemas, y la caja de la unidad en `a_root_as_a_path_and_as_a_file_uri_is_the_same_project`) corrió aquí. Linux lo corre el CI del pull request ([abajo](#ci-del-pull-request)).

## Los tests en rojo

Los de «Cómo se mide» se escribieron antes que cualquier código, con un cliente MCP de prueba que habla por el stdio en memoria del puente (`crates/clawg-cli/src/mcp/mod.rs`), y se corrieron contra el puente de `develop`, al que sólo se le pasó el directorio de arranque y el personal como argumentos (`Start`) para que el test los eligiera.

| Requisito | Test | Cómo fallaba |
|---|---|---|
| BP5 | `a_save_without_a_project_is_refused_with_the_roots_as_options` | El save se guardaba: `saved #1` |
| BP5 | `a_save_without_a_project_is_refused_without_roots_too` | El save se guardaba en el proyecto del directorio de arranque: `saved #1` |
| BP4 | `a_search_with_one_root_reads_its_project` | El puente nunca pedía las raíces y buscaba en el proyecto del directorio de arranque, el personal: no traía la memoria de A |
| BP4 | `a_search_without_roots_reads_the_start_directory` | Pasaba: fija lo que no cambia, un cliente sin roots en un puente arrancado en A |
| BP6, BP7 | `a_search_with_several_roots_reads_all_of_them_and_names_each_project` | `no memories found`: buscaba en el directorio personal |
| BP3 | `the_home_directory_is_never_a_project` | Desde el directorio personal, la búsqueda iba al proyecto «directorio personal» y no traía la memoria de A |
| BP2 | `a_root_as_a_path_and_as_a_file_uri_is_the_same_project` | Con la raíz `c:\…\home\a`, en minúscula como la manda Cursor, no traía lo guardado en `C:\…\home\a` |
| BP1 | `a_change_of_roots_between_two_calls_is_seen_in_the_second` | La primera búsqueda, con la raíz A, ya no traía la memoria de A |
| BP6 | `a_search_takes_a_group_of_projects` (`crates/clawg-cli/tests/api.rs`) | El daemon ignoraba `project_ids` y buscaba en todos los proyectos: volvía la memoria del proyecto de afuera |
| BP6 | `a_group_search_takes_each_row_once` (`search.rs`), `a_group_search_names_its_first_project_for_an_older_daemon` (`client.rs`) | No compilaban: no había `search_projects` ni `grouped` |
| BP11 | `the_protocol_says_where_a_call_without_a_project_goes` (`protocol.rs`) | `Projects names project_path`: la sección decía que las memorias son del directorio en el que se trabaja |
| BP11 | `the_instructions_carry_the_saving_habits`, `the_spanish_instructions_carry_the_saving_habits` | La apertura no pedía nombrar el proyecto de cada save |
| BP7 | `a_group_hit_names_its_project_and_its_key` (`tools.rs`) | No compilaba: no había `format_group_search` |

Los que se escribieron con el código o después se vieron fallar contra una mutación de la línea que los hace pasar, una por test, y la línea se devolvió:

| Requisito | Test | Mutación | Cómo falló |
|---|---|---|---|
| BP5 | `a_save_names_its_project_or_is_refused_before_the_daemon` (`server.rs`) | sin la negativa del save sin lugar | Llegaba al daemon: `the clawg daemon is not running …` |
| BP8 | `what_works_on_one_project_asks_for_one_when_several_are_open` (`server.rs`) | `one_of` toma el primero de varios | `recall_context` respondía `no context yet` |
| BP9 | `a_declared_root_is_indexed_like_the_start_directory` (`server.rs`) | `indexable_roots` sin las raíces | El 403 de `search_project`: la raíz no se había registrado con `index_docs` |
| BP2 | `a_root_is_a_folder_as_a_uri_or_as_a_path` (`roots.rs`) | sin quitar la `/` de `/c:/…` | La URI de VS Code no daba carpeta (`None`): `\c:\Users\u\repo` no es una ruta absoluta de Windows |
| BP3 | `the_home_directory_and_its_root_are_no_folder` (`roots.rs`) | sin comparar la raíz con el directorio personal | Una carpeta de notas bajo un directorio personal con `.git` era una carpeta |
| BP10 | `a_hit_without_a_project_shows_how_a_save_revises_it`, `format_save_shows_the_key_only_where_a_save_revises_it` (`tools.rs`) | `revised_from` como antes | Desde un puente sin proyecto mostraban `topic=style/indent` |

**Tests que cambiaron, y por qué:**

- Los tests de `server.rs` que guardaban sin proyecto nombran el suyo: `save_args` guarda en `PROJ_TEST`, el directorio de arranque de `bridge`, `save_args_in` en la carpeta de otro, y `bare_save` no nombra ninguno, para los que pasan `project` o `all_projects`. Tres sesiones por JSON-RPC (`a_request_without_client_info_keeps_the_name_declared_on_discover`, `a_session_without_initialize_saves_under_the_client_it_declares`, `memories_saved_through_initialize_stay_visible_without_it`) y los tres saves de `crates/clawg-cli/tests/binary.rs` pasan `project_path`.
- `test_project` sale de `ProjectIdentity::resolve("/proj-test")`, el mismo de antes en Linux; en Windows ese camino resuelve a `C:\proj-test`, y hecho a mano no era el proyecto de un save con ese `project_path`. `repo_with_src` deja la ruta sin el `\\?\` de Windows, que ni un directorio de trabajo ni un `project_path` llevan ([tareas](tasks.md#frente-a-lo-previsto)).
- `only_a_folder_the_person_opened_is_indexed` reemplaza a `only_the_bridges_own_root_is_indexed`: `indexes_docs` compara con varias raíces abiertas.
- `resolve_project_resolves_an_explicit_path`, `without_a_path_the_bridges_own_project_is_used`, `all_projects_outranks_everything` y `resolve_project_is_none_when_there_is_nothing_to_scope_to` (`tools.rs`) se borraron con `resolve_project`, la regla del proyecto por defecto.
- `a_hit_without_a_project_shows_how_a_save_revises_it` y `format_save_shows_the_key_only_where_a_save_revises_it` piden lo mismo sin proyecto que con él (BP10).
- `every_route_that_takes_a_project_follows_a_merge` (`crates/clawg-cli/tests/api.rs`) busca también con un grupo que nombra un proyecto unido.
- `a_search_names_its_language_only_when_it_has_one` y el test del cuerpo de `POST /v1/search` (`client.rs`) pasan el grupo vacío a `search_request`.

## La spec contra el código

| Requisito | Dónde |
|---|---|
| BP1 | `declared` (`crates/clawg-cli/src/mcp/roots.rs`), con `ROOTS_WAIT`; `ClawgMcp::at` (`crates/clawg-cli/src/mcp/server.rs`), llamado por `save_memory`, `search_memory`, `recall_context`, `read_memory`, `synthesize`, `search_project`, `answer_suggestion` y `memory_status` |
| BP2 | `root_path`, `from_file_uri`, `percent_decode` e `is_absolute` (`roots.rs`); `folders` resuelve cada una con `Folder::at` |
| BP3 | `Folder::at` (`roots.rs`), para la raíz y para el directorio de arranque (`ClawgMcp::pending`) |
| BP4 | `reading_folders` y `reading` (`server.rs`); `scope_for` |
| BP5 | `unplaced` y su llamada en `save_memory_impl` (`server.rs`), antes de `call` |
| BP6 | `search_projects` (`crates/clawg-core/src/memory/search.rs`); `searched_projects` y `search` (`crates/clawg-cli/src/daemon/api.rs`); `search_group` y `grouped` (`crates/clawg-cli/src/client.rs`); la rama de un grupo en `search_memory_impl` |
| BP7 | `format_group_search` y `hit_lines` (`crates/clawg-cli/src/mcp/tools.rs`) |
| BP8 | `one_of` y `several_open` (`server.rs`), en `recall_context_impl`, `synthesize_impl`, `search_project_impl` y `answer_suggestion_impl`; `save_project` para `memory_status`, `read_memory` y `search_memory` con `all_projects` |
| BP9 | `indexable_roots` y `register` (`server.rs`); `indexes_docs` (`tools.rs`); `not_accepted` (`api.rs`) |
| BP10 | `revised_from` (`tools.rs`) |
| BP11 | `protocol.md` y `protocol.es.md` (la apertura, «How to write a save» / «Cómo escribir un save» y «Projects» / «Proyectos»); las descripciones en `server.rs` y `tools_es.rs`. La apertura mide 2.030 sin preferencia, 2.047 con `en` y 1.938 con `es` |

## El contrato del agente

Sólo cambian las `instructions` (la oración de la apertura) y las descripciones de `project_path` y `project`, en los dos idiomas.

```diff
diff --git a/crates/clawg-cli/src/mcp/agent_contract.json b/crates/clawg-cli/src/mcp/agent_contract.json
@@ -1,5 +1,5 @@
 {
-  "instructions": "… it searches the project's own documentation and code. Memories are scoped to the current project unless you say otherwise.\n\n…",
+  "instructions": "… it searches the project's own documentation and code. Name each save's project: `project_path`, `project` or `all_projects`.\n\n…",
@@ answer_suggestion.project
-            "description": "This project by its id or its name. Defaults to the directory this\nbridge was started in.",
+            "description": "This project by its id or its name. Defaults to the project of the\nfolder open in the client, or of the directory this bridge was started\nin when it names none.",
@@ recall_context.project, search_memory.project, search_project.project, synthesize.project
-            "description": "The project by its id or its name, for one with no folder here or\nanother than this directory's. Not with project_path.",
+            "description": "The project by its id or its name, for one with no folder here or one\nnot open in the client. Not with project_path.",
@@ recall_context.project_path
-            "description": "Absolute path of the project. Defaults to the directory this bridge was\nstarted in.",
+            "description": "Absolute path of the project. Defaults to the project of the folder\nopen in the client, or of the directory this bridge was started in\nwhen it names none; with several open, pass this or project.",
@@ save_memory.project
-            "description": "The project this memory belongs to by its id or its name, for one with\nno folder here, such as an idea, or another than this directory's. A\nname no project has or resembles creates the project. Not with\nproject_path.",
+            "description": "The project this memory belongs to by its id or its name, for one with\nno folder here, such as an idea. A name no project has or resembles\ncreates the project. Not with project_path.",
@@ save_memory.project_path
-            "description": "Absolute path of the project this memory belongs to. Defaults to the\ndirectory this bridge was started in.",
+            "description": "Absolute path of a folder of the project this memory belongs to, such\nas the repository it is about. A save names its project with this,\nproject or all_projects; with none it is refused.",
@@ search_memory.project_path
-            "description": "Absolute path of the project to search in. Defaults to the directory\nthis bridge was started in.",
+            "description": "Absolute path of the project to search in. Defaults to the projects of\nthe folders open in the client, or of the directory this bridge was\nstarted in when it names none.",
@@ search_project.project_path
-            "description": "Absolute path of the project. Defaults to the directory this bridge was\nstarted in. Another project's files are searched only once the user\nhas added it with `clawg projects add`.",
+            "description": "Absolute path of the project. Defaults to the project of the folder\nopen in the client, or of the directory this bridge was started in\nwhen it names none; with several open, pass this or project. Another\nproject's files are searched only once the user has added it with\n`clawg projects add`.",
@@ synthesize.project_path
-            "description": "Absolute path of the project to review. Defaults to the directory this\nbridge was started in.",
+            "description": "Absolute path of the project to review. Defaults to the project of the\nfolder open in the client, or of the directory this bridge was started\nin when it names none; with several open, pass this or project.",
diff --git a/crates/clawg-cli/src/mcp/agent_contract.es.json b/crates/clawg-cli/src/mcp/agent_contract.es.json
@@ -1,5 +1,5 @@
 {
-  "instructions": "… busca en la documentación y el código del propio proyecto. Las memorias son del proyecto actual salvo que digas otra cosa.\n\n…",
+  "instructions": "… busca en la documentación y el código del propio proyecto. Nombra el proyecto de cada save: `project_path`, `project` o `all_projects`.\n\n…",
@@ answer_suggestion.project
-            "description": "Este proyecto por su id o su nombre. Por defecto, el directorio en el que arrancó este puente.",
+            "description": "Este proyecto por su id o su nombre. Por defecto, el proyecto de la carpeta abierta en el cliente, o del directorio en el que arrancó este puente cuando no nombra ninguna.",
@@ recall_context.project, search_memory.project, search_project.project, synthesize.project
-            "description": "Para uno sin carpeta aquí o que no es el de este directorio, el proyecto por su id o su nombre. No con project_path.",
+            "description": "Para uno sin carpeta aquí o que no está abierto en el cliente, el proyecto por su id o su nombre. No con project_path.",
@@ recall_context.project_path
-            "description": "Ruta absoluta del proyecto. Por defecto, el directorio en el que arrancó este puente.",
+            "description": "Ruta absoluta del proyecto. Por defecto, el proyecto de la carpeta abierta en el cliente, o del directorio en el que arrancó este puente cuando no nombra ninguna; con varias abiertas, pasa esta o project.",
@@ save_memory.project
-            "description": "El proyecto de esta memoria por su id o su nombre, para uno sin carpeta aquí, como una idea, o que no es el de este directorio. Un nombre que ningún proyecto tiene ni se le parece crea el proyecto. No con project_path.",
+            "description": "El proyecto de esta memoria por su id o su nombre, para uno sin carpeta aquí, como una idea. Un nombre que ningún proyecto tiene ni se le parece crea el proyecto. No con project_path.",
@@ save_memory.project_path
-            "description": "Ruta absoluta del proyecto al que pertenece esta memoria. Por defecto, el directorio en el que arrancó este puente.",
+            "description": "Ruta absoluta de una carpeta del proyecto al que pertenece esta memoria, como el repositorio del que trata. Un save nombra su proyecto con esta, con project o con all_projects; sin ninguno se rechaza.",
@@ search_memory.project_path
-            "description": "Ruta absoluta del proyecto en el que buscar. Por defecto, el directorio en el que arrancó este puente.",
+            "description": "Ruta absoluta del proyecto en el que buscar. Por defecto, los proyectos de las carpetas abiertas en el cliente, o del directorio en el que arrancó este puente cuando no nombra ninguna.",
@@ search_project.project_path
-            "description": "Ruta absoluta del proyecto. Por defecto, el directorio en el que arrancó este puente. En los archivos de otro proyecto sólo se busca una vez que la persona lo agregó con `clawg projects add`.",
+            "description": "Ruta absoluta del proyecto. Por defecto, el proyecto de la carpeta abierta en el cliente, o del directorio en el que arrancó este puente cuando no nombra ninguna; con varias abiertas, pasa esta o project. En los archivos de otro proyecto sólo se busca una vez que la persona lo agregó con `clawg projects add`.",
@@ synthesize.project_path
-            "description": "Ruta absoluta del proyecto a revisar. Por defecto, el directorio en el que arrancó este puente.",
+            "description": "Ruta absoluta del proyecto a revisar. Por defecto, el proyecto de la carpeta abierta en el cliente, o del directorio en el que arrancó este puente cuando no nombra ninguna; con varias abiertas, pasa esta o project.",
```

El diff completo es el de `crates/clawg-cli/src/mcp/agent_contract.json` y `agent_contract.es.json` en `51410a4f`; aquí las `instructions` se acortan a la oración que cambia y cada hunk se nombra por su herramienta y su argumento.

## `rag-eval`

No aplica. La búsqueda de un proyecto o de todos corre la misma consulta que antes (`search_projects` con un proyecto o ninguno llama a `search_all_memories` como lo hacía `search`), y el agent view busca con un `project_id`, nunca con un grupo, así que ninguna línea que lee cambia.

## CI del pull request

En [#134](https://github.com/MarcossIC/claw-garden/pull/134), sobre `4a6624f9`, pasaron los jobs `test (ubuntu-latest)` y `test (windows-latest)`, que corren la suite y los clippy con `-D warnings`. No hubo nada que arreglar.

## Pendiente en local

Con Cursor real, conectado con `clawg setup cursor` a un perfil con este binario:

- **Un save desde una ventana con un workspace de dos repositorios** cae en el proyecto que nombró el agente: el agente pasa el `project_path` del repositorio del que trata la memoria, y sin él la respuesta lista las dos carpetas. Es lo único que este cambio no puede comprobar sin el cliente: si el modelo de Cursor nombra el proyecto que corresponde.
- **Una búsqueda con dos ventanas abiertas** trae memorias de los dos proyectos, cada una con su `project=`.

## Veredicto

Sin CRITICAL aquí ni en el CI del pull request, en Linux y en Windows. Queda Cursor real, en local ([arriba](#pendiente-en-local)).
