# bridge-project-identity — Verificación

## Suite y clippy

En Linux x86_64, build debug, toolchain 1.99.1, sobre `2f63dd6`, el último commit que toca código o docs (el que sigue sólo toca `openspec/`):

- `cargo test --workspace`: pasa entero, en un `target` limpio; los ignorados son los de antes (los que piden el modelo real o un ensayo a mano).
- `cargo clippy --workspace --all-targets -- -D warnings` sale con 0, sin ningún warning, y `cargo clippy -p clawg-cli -- -D warnings`, lo que construye el release, también. El `#[expect(deprecated)]` de `uris` se cumple: `list_roots` y `ListRootsResult` están marcados obsoletos en `rmcp` 3.5.1.
- `the_agent_contract_matches_its_file` y `the_spanish_agent_contract_matches_its_file` fallaron por la aserción con las descripciones nuevas y pasan después de `CLAWG_BLESS=1`; el diff de los dos contratos va en el pull request.
- En Windows, el CI del pull request ([#133](https://github.com/MarcossIC/claw-garden/pull/133), `windows-latest`). Sobre `09aee6a` fallaron 18 tests de `crates/clawg-cli/src/mcp/server.rs`, todos por cómo los tests armaban un proyecto, no por el puente: `test_project()` armaba a mano la raíz `/proj-test`, que en Windows es una ruta de la unidad actual (`D:\proj-test`, otro id) y que ahora se resuelve porque los saves nombran su carpeta; `repo_with_src` derivaba el id viejo de la forma `\\?\` que da `canonicalize`, y el test del directorio personal comparaba con esa misma forma. `7671fce` arma los tres como el puente resuelve un `project_path`; sobre `7671fce`, Windows pasa los dos clippy y `cargo test --locked --workspace`, y Linux también (`ubuntu-latest`). En Linux, después de ese commit, pasan los tests de `mcp::` y `cargo clippy -p clawg-cli --all-targets -- -D warnings`. Lo `cfg(windows)` que toca este cambio es el de una raíz: `uri_path` saca la barra de antes de la unidad sólo en Windows (`crates/clawg-cli/src/mcp/roots.rs:63`), y `a_root_as_a_path_and_as_a_uri_is_one_project` escribe ahí la ruta con la unidad en minúscula, como la manda Cursor, y la URI con barras; pasó en las dos corridas.

## Los tests en rojo

Escritos antes del código de cada commit.

| Commit | Antes | Después |
|---|---|---|
| `feat(mcp): read a client's roots as the folders it has open` | No compilaba: `roots.rs` sólo tenía sus tests (`cannot find type ProjectIdentity`, `PathBuf`, `Path`, y las funciones) | pasan |
| `feat(search): search a group of projects` | No compilaba: `no method with_projects`, `cannot find function projects_bind`, `count_saves_in_projects` | pasan |
| `feat(daemon): take a group of projects in a search` | `a_search_sends_a_group_only_when_it_has_one` no compilaba (`SearchArgs` sin `project_ids`); con el cliente nuevo y el daemon de antes, `a_search_takes_a_group_of_projects` y `a_search_refuses_a_project_and_a_group` fallaban por la aserción: el daemon ignoraba `project_ids`, buscaba en todos y respondía 200 | pasan |
| `feat(mcp): read the folders the client has open and make every save name its project` | No compilaba: `no method with_roots`, `no field home`, y `format_hits`, `format_search`, `format_save` e `indexes_docs` con la firma de antes | pasan |
| `feat(mcp): tell the model that a save names its project` | Los dos contratos fallaban por la aserción | pasan, bendecidos |

## «Cómo se mide», test por test

| Lo que pedía la exploración | Test |
|---|---|
| un save sin `project`, `project_path` ni `all_projects` se rechaza con las raíces como opciones, con roots | `a_save_that_names_no_project_is_refused_with_the_open_folders` (`server.rs`, dos raíces: el error nombra las dos y los tres argumentos, y el daemon no tiene ninguna fila); el save de `the_roots_are_asked_on_each_call`, por el `duplex` |
| … y sin roots | `a_save_without_roots_offers_the_directory` (el directorio del puente como opción, también en español), `a_save_with_no_open_folder_asks_for_the_project_folder` (sin carpeta: pide la del proyecto) |
| una búsqueda sin proyecto con una raíz busca en su proyecto | `a_search_with_one_root_reads_its_project`, y la primera llamada de `the_roots_are_asked_on_each_call` |
| … con varias busca en todas | `a_search_with_several_roots_reads_all_of_them_and_names_each` (dos de tres checkouts, cada hit con su `project=`); en el core, `a_group_scope_reads_each_of_its_projects` (las tres tablas y el canal léxico); por HTTP, `a_search_takes_a_group_of_projects` |
| … y sin roots usa el directorio | `a_search_without_roots_reads_the_directory`, `a_client_without_roots_is_not_asked` y `a_roots_error_counts_as_no_roots` (por el `duplex`) |
| el directorio personal nunca es un proyecto | `the_home_directory_is_never_a_project` (`roots.rs`: ni como raíz ni como directorio), `a_bridge_started_in_the_home_directory_registers_no_project` (`server.rs`: la búsqueda lee todos los proyectos y no se registra ninguno) |
| una raíz como ruta de Windows y como URI `file:` dan el mismo proyecto | `a_root_is_read_as_a_uri_or_a_path` (puro, con y sin Windows) y `a_root_as_a_path_and_as_a_uri_is_one_project` (una carpeta real, con un espacio en el nombre) |
| un cambio de raíces entre dos llamadas se ve en la segunda | `the_roots_are_asked_on_each_call`: una sesión `initialize` que declara `roots` con `listChanged: false`; la búsqueda ve una carpeta, el cliente abre otra sin avisar, el save rechazado nombra las dos, y el cliente recibió dos `roots/list` |
| un cliente sin la capacidad sigue como hoy | `a_client_without_roots_is_not_asked`: no recibe ningún `roots/list` y la búsqueda lee el directorio del puente |

El cliente de prueba es el `Session` de los tests de `server.rs`, que habla JSON-RPC por un `duplex` como un host, y contesta cada `roots/list` con lo que tiene en ese momento, o con un error.

## El spec contra el código

| Requisito | Dónde |
|---|---|
| BI1 | `roots::asked` y `ROOTS_TIMEOUT` (`crates/clawg-cli/src/mcp/roots.rs:20`, `:25`), `uris` con el `#[expect(deprecated)]` (`roots.rs:44`); `ClawgMcp::asking` (`crates/clawg-cli/src/mcp/server.rs:613`), que usan las ocho herramientas en sus métodos `#[tool]` |
| BI2 | `folder_of`, `is_absolute` y `uri_path` (`roots.rs:52`, `:64`, `:77`) |
| BI3 | `open_folders` (`roots.rs:120`), `ClawgMcp::open_folders` y `canonical_home` (`server.rs:618`, `:531`) |
| BI4 | `SaveMemoryArgs::names_its_project` (`server.rs:208`) en `save_memory_impl` (`server.rs:1139`), `no_project_named` (`crates/clawg-cli/src/mcp/tools.rs:1084`) |
| BI5 | `search_memory_impl` (`server.rs:1217`), `ids_of` (`server.rs:1047`), `save_projects` (`server.rs:1088`); `format_hits`, `revised_from` y `display_project` (`tools.rs:231`, `:179`, `:207`); `SearchArgs.project_ids` y `search_request` (`crates/clawg-cli/src/client.rs:677`, `:702`) |
| BI6 | `memory_project_predicate!`, `table_project_predicate!`, `projects_bind` y `SearchScope::with_projects` (`crates/clawg-core/src/memory/search.rs:119`, `:131`, `:143`, `:60`); `count_saves_in_projects` (`crates/clawg-core/src/memory/language.rs:46`); `SearchRequest.project_ids` y `searched_projects` (`crates/clawg-cli/src/daemon/api.rs:431`, `:1546`) |
| BI7 | `several_open` y `unnamed_with_several` (`server.rs:655`, `:670`), `several_folders` (`tools.rs:1106`), en `recall_context_impl`, `synthesize_impl`, `search_project_impl` y `answer_suggestion_impl` |
| BI8 | `memory_status_impl` con `one_folder` (`server.rs:1662`, `:624`), `read_memory_impl` con `known_ids` (`server.rs:1383`, `:645`) |
| BI9 | `indexable_folders` (`server.rs:631`) e `indexes_docs` (`tools.rs:1073`), en `ids_of`; `not_accepted` (`crates/clawg-cli/src/daemon/api.rs:3171`) |
| BI10 | Los comentarios `///` de los `*Args` y los atributos `#[tool]` de `save_memory` y `search_memory` (`server.rs`), `crates/clawg-cli/src/mcp/tools_es.rs`, la apertura y la sección de proyectos de `protocol.md` y `protocol.es.md`, y los dos contratos |

## Frente a lo previsto

| Punto | Previsto | Hecho | Por qué |
|---|---|---|---|
| Cuándo se piden las raíces | En una llamada sin proyecto | En cada llamada de las ocho herramientas, también la que nombra su proyecto; la spec y la propuesta lo dicen | Una carpeta abierta se registra con su índice de documentos la primera vez que se nombra, y el registro no se repite en el proceso: un save con `project_path` antes de cualquier búsqueda la dejaba sin índice |
| La marca `project=` | En una búsqueda de un grupo | Con varias carpetas abiertas y sin `project` ni `project_path`, también con `all_projects`; la spec lo dice | Es cuando el agente no sabe de qué proyecto es cada hit; con `all_projects` y una carpeta, o nombrando el proyecto, la línea es la de antes |
| `display_project` | No estaba | Un `[` o `]` del id como paréntesis, un control o un salto de línea como espacio; test `a_project_named_by_a_hit_keeps_the_hit_on_its_line` | El id sale del nombre de una carpeta, y uno con `]` cerraría el encabezado del hit, que `rag-eval` y el agente leen hasta el primer `]` |
| `SearchScope` | «Guarda la lista» | El campo público pasa de `project_id` a `projects`; los cuatro tests que lo leían (`search.rs` y `tests/public_api.rs`) leen el nuevo, y los que llaman a las funciones internas de búsqueda con un proyecto lo atan con `bound` | El campo es la lista; lo que esperan esos tests no cambió |
| `count_saves_in` | Contar un grupo | Queda como estaba, para un proyecto, y suma `count_saves_in_projects` | Su test no cambia, y el contexto sigue contando un proyecto |
| Los tests que guardaban sin proyecto | `save_args` pasa `project_path` | `save_here`, un método de los tests de `server.rs`, nombra la carpeta del puente, o `all_projects` en uno sin carpeta, cuando el save no nombra nada; los tres saves de `tests/binary.rs` y los del `duplex` pasan `all_projects` o `project_path` | Cada test guarda donde guardaba antes, con la carpeta del puente que hasta ahora ponía el puente |
| README | Fuera del alcance | La lista de herramientas MCP y la sección de Cursor dicen que un save nombra su proyecto y qué hace con las carpetas abiertas | Es lo que lee quien instala; `docs/reference/daemon.md` lo dice para quien mantiene |
| Las firmas de `clawg-eval` y `contradiction.rs` | Fuera del alcance | `rag-eval` y `revision-eval` pasan a `format_search` una lista (`&[]`, o la del proyecto que pasaban), y arman `SearchArgs` con `project_ids` vacío; `contradiction.rs` arma su `SearchScope` con `projects` | Son los llamadores de lo que cambió de firma; lo que hacen no cambia |
| Los tests en Windows | No estaba | `7671fce`: `test_project()`, `repo_with_src` y el test del directorio personal arman sus carpetas como el puente resuelve un `project_path` | Con todo save nombrando su carpeta, la ruta de un proyecto de prueba se resuelve; armada a mano (`/proj-test`, o la forma `\\?\` de `canonicalize`) daba otro id en Windows |
| `resolve_project` | No se nombraba | Sigue, con la única carpeta abierta como proyecto por defecto | Su orden (`all_projects`, después la ruta, después el proyecto por defecto) es el mismo, y sus tests no cambian |

## Lo que trajo la otra aplicación

El mismo cambio se aplicó también en otra rama ([#134](https://github.com/MarcossIC/claw-garden/pull/134)), con otro diseño en dos lugares. Se compararon por partes y queda esta:

| Parte | Esta | La otra | Por qué queda esta |
|---|---|---|---|
| La búsqueda de un grupo | Una búsqueda sobre la unión de los proyectos: el predicado ata una lista JSON | Una búsqueda por proyecto, juntadas por puntaje (`search_projects`) | Un ranking con un solo `now` y los cupos de candidatos compartidos; una pasada en lugar de una por proyecto, y las métricas de latencia una vez por pedido; el grupo cuenta el otro idioma, que la otra deja afuera |
| El puente | `open_folders` puro, con el resolvedor y el directorio personal como argumentos; las herramientas de un proyecto se niegan antes de llamar al daemon | Las carpetas se resuelven con I/O, y las de un grupo se registran antes de negarse | Lo que pide `.claude/CLAUDE.md` de separar la decisión del I/O, y una negativa sin efectos |
| Los tests del puente | Un cliente por `duplex` que cuenta los `roots/list` y prueba un error de `roots/list`, `memory_status` y `read_memory` con varias carpetas | Un cliente por `duplex` que maneja el `run_with` entero | Cubren lo mismo de BI1 a BI5, y esta además los caminos de error |

De la otra se trajo lo que esta no tenía, en un commit cada punto:

| Commit | Antes | Después |
|---|---|---|
| `fix(mcp): show the key of a row without a project only as all_projects` | `a_hit_without_a_project_shows_how_a_save_revises_it` y `format_save_shows_the_key_only_where_a_save_revises_it` fallaban por la aserción: sin carpetas abiertas, una fila sin proyecto mostraba ` topic=K`, y un save sin proyecto se rechaza y uno con `project_path` deja otra versión al lado | pasan |
| `fix(search): let an older daemon read a group as its first project` | `a_group_search_names_its_first_project_for_an_older_daemon` y `a_group_wins_over_the_project_of_a_search` fallaban por la aserción: el pedido no llevaba `project_id`, así que un daemon anterior buscaba en todos, y el daemon respondía 400 a los dos juntos | pasan; `every_route_that_takes_a_project_follows_a_merge`, que pasa también con `project_ids`, cubre el grupo |
| `fix(mcp): read a Windows share as a root, and home inside a checkout as home` | No compilaba: `folder_of` no tomaba el sistema | pasan |
| `fix(daemon): say that a folder the client has open is read without adding it` | `the_refusal_names_what_is_read_without_the_user_adding_it` fallaba por la aserción: el 403 decía que sólo se lee el directorio en el que arrancó un puente | pasa |
| `docs(mcp): name all_projects in the opening, and the key of a group's hit` | `the_protocol_says_where_a_call_without_a_project_goes` fallaba por la aserción: la sección de proyectos escribía project= sin comillas y no nombraba `answer_suggestion`, y la apertura no nombraba `all_projects`; los dos contratos fallaron por la aserción con la apertura nueva | pasan, bendecidos; los contratos cambian sólo en esa oración |

Lo que no se trajo:

- `search_projects`, por lo de la tabla de arriba.
- `ROOTS_WAIT` de 3 s: ninguno de los dos plazos se midió, y 2 s ya está en la spec y en la documentación.
- Contar «varias carpetas» por proyecto registrado en lugar de por raíz: dos clones de un mismo remoto cuentan como dos carpetas para `recall_context`, pero contarlos como uno pide registrarlos antes de negarse, que es lo que esta evita.
- El cliente de prueba que maneja `run_with`, con un `Start` inyectado: pide hacer públicos `Start`, `Folder` y `with_roots` sólo para los tests, y los escenarios que prueba ya los prueban los tests de `server.rs`.

## Pendiente en local

Con Cursor instalado (la prueba de la exploración usó 3.28.7 en Windows) y un clawg con este cambio conectado con `clawg setup cursor`: un build de esta rama con el perfil `dev`, o la versión publicada con `prod`. Después de reiniciar Cursor:

1. **Un save desde Cursor en un workspace con dos repositorios cae en el proyecto correcto.** Con un workspace de dos repositorios abierto en una ventana, pedirle al agente que guarde algo de uno de ellos. Si el agente no nombra el proyecto, la respuesta de `save_memory` es el rechazo con las dos carpetas; el agente vuelve a guardar con `project_path`. `clawg memories list --profile P` muestra la fila en el proyecto de ese repositorio, y `clawg projects list --profile P` no lista el directorio personal.
2. **Una búsqueda con dos ventanas abiertas trae memorias de los dos repositorios,** cada hit con su `project=`.
3. **Que Cursor conteste `roots/list` a tiempo en cada llamada:** el log del daemon no dice `the client did not list its roots within 2 s`.
4. **Claude Code:** con una carpeta, lo que lee es lo de antes, declare `roots` o no; un save sin proyecto se rechaza y el agente lo repite con `project_path`.

## Veredicto

Sin CRITICAL en Linux ni en el CI de Windows. Los puntos de «Pendiente en local» quedan como WARNING hasta probarlos con Cursor.
