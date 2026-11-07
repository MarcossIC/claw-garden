# bridge-project-identity — Especificación

De qué proyecto lee y en cuál guarda el puente `clawg mcp` cuando una herramienta no nombra uno. Al archivar, BI1 a BI5 y BI7 a BI10 se suman a [`specs/mcp`](../../../specs/mcp/spec.md) y BI6 a [`specs/retrieval`](../../../specs/retrieval/spec.md), y reemplazan lo que dicen del proyecto por defecto del puente:

- de [`specs/project`](../../../specs/project/spec.md), R3 en lo que deja al puente sin otro proyecto que el de su `current_dir()` (y su escenario, un save sin `project_path` que cae en el proyecto del directorio), y la regla de `index_docs` («`index_docs` MUST ser `true` sólo cuando la raíz de la llamada es la raíz del puente»);
- de `specs/mcp`, la última frase de PM4 («Ausente o `false`, el save MUST guardarse en el proyecto que resuelve sin él»), la de PM8 («un puente sin proyecto MUST seguir viendo ` topic=K`») y, en «La clave de tema en las herramientas», «salvo que el save tampoco tenga»;
- de [`specs/project-docs`](../../../specs/project-docs/spec.md), que sólo la raíz del puente se acepta sola.

Lo que esas specs dicen de una herramienta que nombra su proyecto no cambia.

## Definiciones

- **Cliente con raíces:** uno que declara la capacidad `roots`: en `initialize` (ciclo legacy) o en `io.modelcontextprotocol/clientCapabilities` del `_meta` de la request (ciclo 2026-07-28). Lo lee `RequestContext::client_capabilities` de `rmcp`.
- **Llamada sin proyecto:** una herramienta llamada sin `project` ni `project_path`.
- **Directorio personal:** el de `clawg_core::paths::home_dir` (el de `CLAWG_TEST_HOME` en un build de tests), canónico.
- **Carpetas abiertas** de una llamada: las de BI3.
- **Grupo:** dos o más proyectos que una búsqueda lee a la vez.

## BI1. Las raíces se piden en cada llamada

Ante cada llamada de `save_memory`, `search_memory`, `recall_context`, `search_project`, `synthesize`, `read_memory`, `answer_suggestion` o `memory_status`, también la que nombra su proyecto, el puente MUST mandar `roots/list` a un cliente con raíces, dentro de esa llamada, y esperar la respuesta como mucho `ROOTS_TIMEOUT` (2 s). MUST NOT reusar la respuesta en otra llamada, ni esperar `notifications/roots/list_changed`. A un cliente sin raíces MUST NOT mandarle `roots/list`. Un error, un plazo vencido o una respuesta sin carpetas MUST contar como un cliente sin raíces. `list_memories`, `delete_memory`, `restore_memory`, `update_memory` y `resolve_conflict` MUST NOT pedir raíces.

#### Escenario: las raíces cambian entre dos llamadas
- GIVEN una sesión `initialize` que declara `roots`, con `/work/a` abierta
- WHEN busca sin proyecto, el cliente abre además `/work/b` sin mandar `list_changed`, y guarda sin proyecto
- THEN la búsqueda lee el proyecto de `/work/a`, el save se rechaza nombrando `/work/a` y `/work/b`, y el cliente recibió dos `roots/list`

#### Escenario: un cliente sin raíces
- GIVEN una sesión que no declara `roots` y un puente que arrancó en `/proj-test`
- WHEN busca sin proyecto
- THEN el cliente no recibe ningún `roots/list` y la búsqueda lee el proyecto de `/proj-test`

**Tests:** `the_roots_are_asked_on_each_call`, `a_client_without_roots_is_not_asked`, `a_roots_error_counts_as_no_roots` (`server.rs`).

## BI2. Una raíz es una carpeta

El `uri` de una raíz MUST leerse así: un `file:` con host vacío o `localhost` es el camino que sigue, con los `%XX` decodificados como UTF-8, y en Windows sin la barra que precede a la unidad; un `file:` con otro host MUST leerse en Windows como la carpeta compartida que nombra (`file://server/share` es `\\server\share`) e ignorarse en los demás sistemas; uno que no decodifica o un esquema que no es `file:` MUST ignorarse; cualquier otro texto es un camino si es absoluto en el sistema, y si no MUST ignorarse. En Windows cada `/` MUST quedar `\`. La función MUST ser pura, también en el sistema que lee, que se le pasa, así las formas de cada uno se prueban en cualquiera. Cada carpeta MUST resolverse con `ProjectIdentity::resolve`, como un `project_path`.

#### Escenario: Cursor en Windows
- GIVEN una carpeta `C:\work\repo` que existe
- WHEN una raíz llega como `c:\work\repo` y otra como `file:///C:/work/repo`
- THEN las dos dan el mismo proyecto

**Tests:** `a_root_is_read_as_a_uri_or_a_path`, `a_root_is_read_as_each_system_reads_it`, `a_root_as_a_path_and_as_a_uri_is_one_project` (`roots.rs`; el segundo con una carpeta temporal, que en Windows escribe la ruta con la unidad en minúscula y la URI con barras).

## BI3. Las carpetas abiertas

Las carpetas abiertas de una llamada MUST ser las raíces del cliente, resueltas, en su orden y sin repetir una raíz; si no queda ninguna, el proyecto del directorio donde arrancó el puente; si tampoco, ninguna. **El directorio personal MUST NOT ser un proyecto:** una carpeta que es el directorio personal, o cuya raíz lo es, MUST quitarse, venga del cliente o del directorio del puente: el directorio personal puede estar dentro de un checkout, cuya raíz es otra. La decisión MUST ser pura, con el resolvedor y el directorio personal como argumentos.

#### Escenario: el puente arranca en el directorio personal
- GIVEN un puente que arrancó en el directorio personal y un cliente sin raíces
- WHEN busca sin proyecto
- THEN la búsqueda lee todos los proyectos y el daemon no registra ninguno

**Tests:** `the_open_folders_are_the_roots_else_the_directory`, `the_home_directory_is_never_a_project` (`roots.rs`); `a_bridge_started_in_the_home_directory_registers_no_project` (`server.rs`).

## BI4. Un save nombra su proyecto

`save_memory` sin `project`, `project_path` ni `all_projects: true` MUST negarse sin llamar al daemon. La frase MUST estar en el idioma de la sesión, nombrar los tres argumentos, y listar cada carpeta abierta como su raíz, para pasarla en `project_path`; sin carpetas abiertas, MUST decir que se pase la carpeta del proyecto. Con uno de los tres, el save MUST hacer lo de antes.

#### Escenario: dos carpetas abiertas
- GIVEN un cliente con `/work/a` y `/work/b` abiertas
- WHEN guarda sin proyecto
- THEN la respuesta es un error que empieza por `not saved:`, nombra `/work/a`, `/work/b`, `project_path`, `project` y `all_projects`, y el daemon no tiene ninguna fila

**Tests:** `a_save_that_names_no_project_is_refused_with_the_open_folders`, `a_save_without_roots_offers_the_directory`, `a_save_with_no_open_folder_asks_for_the_project_folder` (`server.rs`), y el save de `the_roots_are_asked_on_each_call`.

## BI5. `search_memory` lee las carpetas abiertas

Una llamada sin proyecto ni `all_projects` MUST buscar: sin carpetas abiertas, en todos los proyectos; con una, en su proyecto, registrado como hoy el del directorio; con varias, en el grupo de sus proyectos, cada uno registrado (dos carpetas del mismo proyecto cuentan una vez). Cuando una llamada sin `project` ni `project_path` tiene varias carpetas abiertas (un grupo, o todos con `all_projects`), cada hit cuya fila tiene proyecto MUST mostrar ` project=<id>` después del puntaje y de la clave, con un `[` o un `]` del id como paréntesis y un control o un salto de línea como espacio; un hit sin proyecto MUST NOT mostrarlo. Con una carpeta o ninguna, o con `project` o `project_path`, cada hit MUST mostrarse como antes. `topic=K` MUST mostrarse en una fila de cualquier proyecto de las carpetas abiertas (`revised_from` contra una lista: con un proyecto, la regla de antes), y con `all_projects` la lista es la de las carpetas abiertas. Una fila sin proyecto MUST NOT mostrar ` topic=K` desde ningún puente, con carpetas abiertas o sin ellas, porque un save nombra su proyecto y sólo uno con `all_projects` la revisa: una `user_preference` sin proyecto MUST mostrar ` topic=K all_projects`, y cualquier otra, ninguna clave; un hit sin `project_id`, de un daemon anterior al campo, MUST mostrarla como antes. El puente MUST mandar el grupo en `project_ids` y, en `project_id`, su primer proyecto: un daemon anterior a los grupos ignora `project_ids` y busca en ese, más estrecho que el grupo y nunca en todos.

#### Escenario: un workspace de dos repositorios
- GIVEN una memoria en el proyecto de `/work/a`, otra en el de `/work/b` y otra en el de `/work/c`, y un cliente con `/work/a` y `/work/b` abiertas
- WHEN busca sin proyecto algo que las tres responden
- THEN vuelven las dos primeras, cada una con su `project=`, y no la tercera

**Tests:** `a_search_with_one_root_reads_its_project`, `a_search_with_several_roots_reads_all_of_them_and_names_each`, `a_search_without_roots_reads_the_directory` (`server.rs`); `format_hits_names_the_project_of_each_hit_of_a_group`, `a_project_named_by_a_hit_keeps_the_hit_on_its_line`, `the_key_shows_on_a_row_of_any_open_folder`, `a_hit_without_a_project_shows_how_a_save_revises_it`, `format_save_shows_the_key_only_where_a_save_revises_it` (`tools.rs`); `a_search_sends_a_group_only_when_it_has_one`, `a_group_search_names_its_first_project_for_an_older_daemon` (`client.rs`).

## BI6. La búsqueda de un grupo

El predicado de proyecto (`memory_project_predicate!`, y el de `episodic_trajectory` y `procedural_skill` en las consultas de búsqueda) MUST atar una lista JSON de ids o NULL: con NULL, todas las filas; con una lista, las de cada proyecto de la lista, las sin proyecto y, en `memory`, toda `user_preference`. Un proyecto MUST atarse como una lista de uno, en cada consulta que usa el predicado. `SearchScope` MUST guardar la lista (`with_project` para uno, `with_projects` para varios). `POST /v1/search` MUST tomar `project_ids`, una lista de ids que pasan cada uno por `live_project`; con `project_id` y `project_ids` a la vez MUST leer el grupo, y una lista vacía MUST responder 400. El conteo del otro idioma de esa búsqueda MUST contar el mismo grupo. Lo que lee una consulta con un proyecto MUST NOT cambiar.

#### Escenario: un grupo en el core
- GIVEN filas en los proyectos `p`, `q` y `r`, una sin proyecto y una `user_preference` de `r`
- WHEN se busca con el grupo `p`, `q`
- THEN vuelven las de `p` y `q`, la sin proyecto y la preferencia, y no el resto de `r`

**Tests:** `a_group_scope_reads_each_of_its_projects` (`search.rs`), `counting_a_group_counts_each_of_its_projects` (`language.rs`), `a_search_takes_a_group_of_projects`, `a_group_wins_over_the_project_of_a_search` y, con un proyecto unido a otro, `every_route_that_takes_a_project_follows_a_merge` (`crates/clawg-cli/tests/api.rs`). Los tests del core que buscan, cuentan pares o arman un contexto con un proyecto pasan sin cambiar lo que esperan, y los de `EXPLAIN QUERY PLAN` siguen viendo la tabla que manda.

## BI7. Las herramientas de un proyecto

Con varias carpetas abiertas, una llamada sin proyecto de `recall_context`, de `synthesize` sin `all_projects`, de `search_project` y de `answer_suggestion` MUST negarse sin leer, en el idioma de la sesión, listando cada carpeta como su raíz para `project_path` (o `project` en `answer_suggestion`, que no toma camino) y diciendo que `search_memory` busca en todas. Con una carpeta MUST hacer lo de antes con el directorio del puente; sin ninguna, lo de antes sin proyecto.

#### Escenario: `recall_context` en un workspace de dos repositorios
- GIVEN un cliente con `/work/a` y `/work/b` abiertas
- WHEN llama a `recall_context` sin proyecto
- THEN la respuesta es un error que nombra `/work/a`, `/work/b`, `project_path` y `search_memory`

**Tests:** `a_one_project_tool_asks_which_folder_when_several_are_open` (`server.rs`, las cuatro herramientas).

## BI8. `memory_status` y `read_memory`

Los pares de `memory_status` MUST ser los del proyecto de la carpeta abierta cuando hay una, y los de todos los proyectos cuando hay varias o ninguna. `read_memory` MUST mostrar `topic=K` con la regla de BI5 contra los proyectos de las carpetas abiertas.

**Tests:** `memory_status_counts_every_project_with_several_folders`, `read_memory_shows_the_key_of_a_row_in_any_open_folder` (`server.rs`).

## BI9. Los documentos de las carpetas abiertas

Una carpeta abierta MUST registrarse con `index_docs` si `auto_indexable` la acepta (tiene `.git` o un manifiesto y no es el directorio personal), como hoy el directorio del puente. Un camino que nombra el modelo y no es una carpeta abierta MUST seguir registrándose sin `index_docs`. El 403 de `search_project` (`not_accepted`, `crates/clawg-cli/src/daemon/api.rs`), `clawg projects add` en `docs/reference/commands.md` y `docs/reference/daemon.md` MUST decir que se lee sola una carpeta que el cliente MCP tiene abierta o en la que arrancó un puente, y no sólo esta.

**Tests:** `a_root_the_client_declares_is_indexed_like_the_directory` (`server.rs`), `the_refusal_names_what_is_read_without_the_user_adding_it` (`api.rs`), `only_an_open_folder_is_indexed` (`tools.rs`, en lugar de `only_the_bridges_own_root_is_indexed`); `a_save_elsewhere_registers_the_root_without_reading_it` y `a_bridge_in_a_folder_that_is_not_a_project_indexes_nothing` siguen pasando.

## BI10. Lo que ve el modelo

- La descripción de `project_path` de `save_memory` MUST decir que no tiene valor por defecto y que un save pasa `project_path`, `project` o `all_projects`; las de `project_path` de las herramientas que leen, que sin él leen las carpetas que el cliente tiene abiertas, o el directorio del puente.
- La de `search_memory` MUST decir que con varias carpetas busca en todas y que cada hit nombra su proyecto con `project=`, que es lo que `project` acepta.
- La apertura de `protocol.md` y de `protocol.es.md`, lo que lee un cliente anterior, MUST pedir que cada save nombre su proyecto con `project_path`, `project` o `all_projects`. Su sección de proyectos MUST decir cuándo pasar `project_path` y que, con varias carpetas abiertas, `recall_context`, `search_project`, `synthesize` y `answer_suggestion` piden en cuál se trabaja; «How to write a save» y «Cómo escribir un save», que un hit de una búsqueda en varios proyectos muestra su clave junto a su `project=`, y que un save con ese `project` la revisa. Cada variante de la apertura MUST seguir cabiendo en 2048 caracteres.
- `project=` se suma a los marcadores ([`specs/mcp`](../../../specs/mcp/spec.md), «Definiciones»).
- `agent_contract.json` y `agent_contract.es.json` MUST coincidir con lo que lista el servidor, bendecidos con `CLAWG_BLESS=1`.

**Tests:** `the_agent_contract_matches_its_file`, `the_spanish_agent_contract_matches_its_file`, los del largo de la apertura, `the_protocol_names_only_what_exists` y `the_protocol_says_where_a_call_without_a_project_goes` (`protocol.rs`, en los dos idiomas).

## Verificación

En Linux, con directorios temporales, el embedder falso y un cliente MCP de prueba por un `duplex`. El código `cfg(windows)`, en el CI del pull request. Lo que sólo Cursor reproduce queda en [`verify.md`](verify.md) como pendiente en local.
