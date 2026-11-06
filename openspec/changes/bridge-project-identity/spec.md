# bridge-project-identity — Especificación

Cambia de [`specs/project`](../../specs/project/spec.md) R3, que deja al puente sin otro proyecto por defecto que el de su `current_dir()`, la línea de PL47 sobre el camino crudo del puente, y la regla de `index_docs` («`index_docs` MUST ser `true` sólo cuando la raíz de la llamada es la raíz del puente»). Cambia de [`specs/mcp`](../../specs/mcp/spec.md) «La clave de tema en las herramientas», la última frase de PM8, PM4 («Ausente o `false`, el save MUST guardarse en el proyecto que resuelve sin él»), «Un proyecto por su id o su nombre» («Sin el argumento, cada herramienta hace lo de antes»), «Las instrucciones caben en 2.048 caracteres» y la nota de «Mover una memoria de proyecto» sobre `project_path`. Suma a `specs/mcp` las raíces del cliente, el save que nombra su proyecto y la búsqueda en un grupo, y a [`specs/project-docs`](../../specs/project-docs/spec.md) la raíz declarada que se indexa. Al archivar, estos requisitos pasan a esas specs.

## Definiciones

- **Raíz:** una entrada de la respuesta a `roots/list` del cliente MCP, con su `uri`.
- **Directorio de arranque:** el `current_dir()` del proceso del puente al arrancar.
- **Directorio personal:** el que da `clawg_core::paths::home_dir()`, canónico.
- **Carpetas de lectura de una llamada:** las raíces de esa llamada leídas como carpetas (BP2) sin el directorio personal (BP3), si queda alguna; si no, el directorio de arranque, si no es el personal; si no, ninguna.
- **Proyectos de lectura:** los proyectos que da registrar cada carpeta de lectura (`ids_for`), sin repetir. Uno es un **proyecto**; dos o más, un **grupo**.

## BP1. El puente pide las raíces en cada llamada

En cada llamada a una herramienta que lee o guarda (`save_memory`, `search_memory`, `recall_context`, `read_memory`, `synthesize`, `search_project`, `answer_suggestion` y `memory_status`), el puente MUST pedir `roots/list` si el cliente declaró la capacidad `roots` (`RequestContext::client_capabilities`, que lee el `initialize` o el `_meta` de la llamada), y MUST NOT pedirla si no la declaró. MUST esperar la respuesta como mucho `ROOTS_WAIT` (3 s). Una respuesta de error, o ninguna a tiempo, MUST contar como un cliente sin roots en esa llamada, con un warning en el log, y la herramienta MUST seguir. Las raíces MUST pedirse otra vez en la llamada siguiente: nunca se guardan entre llamadas, porque un cliente puede cambiarlas sin avisar (Cursor declara `roots.listChanged: false`).

#### Escenario: las raíces cambian entre dos llamadas
- GIVEN un cliente con roots que declara la carpeta A, con una memoria guardada en A y otra en B
- WHEN busca sin proyecto, el cliente pasa a declarar B sin mandar `notifications/roots/list_changed`, y busca otra vez
- THEN la primera búsqueda trae la memoria de A y no la de B, y la segunda la de B y no la de A

#### Escenario: un cliente sin roots
- GIVEN un cliente que no declara la capacidad `roots`
- WHEN llama a cualquier herramienta
- THEN el puente no le manda `roots/list`

Las demás (`list_memories`, `delete_memory`, `restore_memory`, `update_memory`, `resolve_conflict`) no dependen del proyecto de la llamada y no las piden.

**Tests:** `a_change_of_roots_between_two_calls_is_seen_in_the_second` (`crates/clawg-cli/src/mcp/mod.rs`); el cliente de prueba falla si recibe `roots/list` sin haber declarado la capacidad, así que cada test sin roots lo comprueba. Hoy el primero falla: el puente nunca pide las raíces y busca en su directorio de arranque.

## BP2. Una raíz es una carpeta

Leer una raíz como carpeta MUST ser una función pura de su `uri` y del sistema (`root_path`, `crates/clawg-cli/src/mcp/roots.rs`):

- `file://` seguido de una ruta absoluta: la ruta, sin el host `localhost` y con cada `%XX` decodificado. En Windows, `/c:/x` MUST quedar `c:\x`, y un host que no es `localhost` MUST dar la ruta UNC `\\host\x`; las `/` pasan a `\`.
- Una ruta absoluta que no es una URI, como la que manda Cursor (`c:\x`): esa ruta.
- Cualquier otra cosa (una ruta relativa, otro esquema, un `%` que no es un byte, una ruta que no es UTF-8): ninguna carpeta, y la raíz se descarta.

Cada carpeta MUST resolver su proyecto con `ProjectIdentity::resolve`, como un `project_path`.

#### Escenario: Cursor y una URI
- GIVEN una carpeta que es un repositorio
- WHEN el cliente la declara una vez como ruta y otra como `file:` con su ruta codificada (en Windows, la ruta con la unidad en minúscula y la URI con `%3A`)
- THEN las dos llevan al mismo proyecto, y una búsqueda trae lo guardado con su `project_path`

**Tests:** `a_root_is_a_folder_as_a_uri_or_as_a_path` y `a_root_that_names_no_folder_here_is_dropped` (`roots.rs`), puros, con las formas de Windows y de Unix en cualquier sistema; `a_root_as_a_path_and_as_a_file_uri_is_the_same_project` (`mod.rs`). Hoy no compilan: no hay `root_path`.

## BP3. El directorio personal nunca es un proyecto

Una carpeta cuya ruta canónica, o cuya raíz (`ProjectIdentity.root`), es el directorio personal MUST NOT ser una carpeta de lectura, venga del directorio de arranque o de una raíz, y MUST NOT registrarse por eso. Una carpeta dentro del directorio personal MUST seguir siendo una carpeta de lectura.

#### Escenario: Cursor arranca en el directorio personal
- GIVEN un puente cuyo directorio de arranque es el personal, con un cliente sin roots
- WHEN busca sin proyecto y guarda sin proyecto
- THEN la búsqueda va sin proyecto, el save se niega sin ofrecer el directorio personal, y el directorio personal no se registra como proyecto
- WHEN el cliente declara como raíces el directorio personal y la carpeta A
- THEN la búsqueda va sólo a A

**Tests:** `the_home_directory_is_never_a_project` (`mod.rs`) y `the_home_directory_and_its_root_are_no_folder` (`roots.rs`). Hoy el primero falla: el directorio personal es el proyecto por defecto.

## BP4. Dónde lee una llamada sin proyecto

Una herramienta que lee sin `project`, `project_path` ni `all_projects` MUST leer en sus proyectos de lectura: con un proyecto, en él, como hoy en el del directorio de arranque; sin ninguno, sin proyecto, como hoy un puente sin directorio. Cada carpeta de lectura MUST registrarse una vez por raíz y camino en el proceso, como cualquier camino que nombra una herramienta.

#### Escenario: una raíz, y ninguna
- GIVEN memorias guardadas con el `project_path` de A y con el de B
- WHEN un cliente con la raíz A busca sin proyecto
- THEN trae la de A y no la de B
- WHEN un cliente sin roots, en un puente arrancado en A, busca sin proyecto
- THEN trae la de A y no la de B

**Tests:** `a_search_with_one_root_reads_its_project` y `a_search_without_roots_reads_the_start_directory` (`mod.rs`). Hoy el primero falla: el puente busca en su directorio de arranque.

## BP5. Un save nombra su proyecto

`save_memory` sin `project_path`, `project` ni `all_projects` (este último en `true`) MUST negarse sin llamar al daemon, con un error en el idioma de la sesión que nombra los tres argumentos y lista, como opciones de `project_path`, la ruta de cada carpeta de lectura; sin carpetas de lectura, sólo los argumentos. Nunca MUST guardar en un proyecto que la llamada no nombró. Con uno de los tres, el save MUST hacer lo de hoy.

#### Escenario: con roots y sin roots
- GIVEN un cliente con las raíces A y B
- WHEN guarda sin `project`, `project_path` ni `all_projects`
- THEN la respuesta es un error que nombra `project_path`, `project` y `all_projects` y lista A y B, y no se guarda nada
- GIVEN un cliente sin roots en un puente arrancado en A
- WHEN guarda igual
- THEN el error lista A, y no se guarda nada

**Tests:** `a_save_without_a_project_is_refused_with_the_roots_as_options` y `a_save_without_a_project_is_refused_without_roots_too` (`mod.rs`); `a_save_names_its_project_or_is_refused_before_the_daemon` (`server.rs`), contra un daemon que no responde. Hoy fallan: el save se guarda en el proyecto del directorio.

## BP6. `search_memory` busca en un grupo

Con un grupo, `search_memory` sin `project`, `project_path` ni `all_projects` MUST buscar en cada proyecto del grupo. `POST /v1/search` MUST tomar `project_ids`, una lista opcional de ids:

- Con dos o más ids distintos, después de seguir cada uno por `canonical`, el daemon MUST embeber la consulta una vez, correr para cada proyecto la búsqueda de un proyecto (`search_all_memories` con su `SearchScope`), juntar los hits por puntaje, dejar el primero de cada fila (tabla e id) y de cada save, cortar en `limit` y aplicar `min_relative_score` sobre el resultado junto, antes de registrar los accesos (`search_projects`, `crates/clawg-core/src/memory/search.rs`). MUST NOT contar `other_language`.
- Con un solo id distinto, MUST ser la búsqueda de ese proyecto. Sin `project_ids`, o vacía, MUST ser la de `project_id`, como hoy.

El puente MUST mandar con el grupo también `project_id`, con su primer proyecto: un daemon anterior ignora `project_ids` y busca en ese, nunca en todos.

#### Escenario: tres proyectos
- GIVEN una memoria en el proyecto A, una en B, una en C y una sin proyecto, cerca de la consulta
- WHEN se busca con `project_ids: [A, B]`
- THEN vuelven las de A, B y la sin proyecto, esta una sola vez, y no la de C
- AND con `project_ids: [A]` vuelve lo mismo que con `project_id: A`

**Tests:** `a_group_search_takes_each_row_once` (`search.rs`); `a_search_takes_a_group_of_projects` (`crates/clawg-cli/tests/api.rs`); `a_group_search_names_its_first_project_for_an_older_daemon` (`client.rs`); `a_search_with_several_roots_reads_all_of_them_and_names_each_project` (`mod.rs`). Hoy no compilan: no hay `project_ids` ni `search_projects`.

## BP7. Cada hit de un grupo dice su proyecto

En una búsqueda en grupo, la línea de un hit de una fila con proyecto MUST llevar ` project=<id>` después de `score=S`, con el id que manda el daemon, y su clave como ` topic=K`, porque un save con ese `project` y esa clave la revisa: `N. #id [kind/tipo/tN score=S project=P topic=K by A] contenido`. Un hit de una fila sin proyecto MUST NOT llevar `project=`, y su clave MUST seguir BP10. La línea de una búsqueda en un proyecto, o en todos, MUST quedar byte a byte como hoy.

**Tests:** `a_group_hit_names_its_project_and_its_key` (`crates/clawg-cli/src/mcp/tools.rs`); los tests de `format_hits` que ya existen fijan la línea de un proyecto. Hoy no compila: `format_search` no sabe de grupos.

## BP8. Lo que es de un proyecto pide uno cuando hay varios

`recall_context`, `synthesize`, `search_project` y `answer_suggestion` sin `project`, `project_path` (en `answer_suggestion`, sin `project`) ni `all_projects` MUST, con un grupo, negarse sin buscar ni escribir memorias (las carpetas de lectura se registran, como en cualquier lectura), con un error en el idioma de la sesión que dice que el cliente tiene varios proyectos abiertos y lista la ruta de cada carpeta de lectura como opción de `project_path`, o `project`. Con un proyecto MUST usarlo; sin ninguno, MUST hacer lo de hoy (`recall_context` y `synthesize` sin proyecto; `search_project` y `answer_suggestion` se niegan con su frase). `memory_status` MUST contar los conflictos del proyecto de lectura, o de todos los proyectos sin uno o con un grupo. La clave que muestran `read_memory` y `search_memory` con `all_projects` MUST compararse con el proyecto de lectura, o con ninguno sin uno o con un grupo.

#### Escenario: dos ventanas
- GIVEN un cliente con las raíces A y B, de dos proyectos
- WHEN llama a `recall_context`, `synthesize`, `search_project` o `answer_suggestion` sin proyecto
- THEN cada una responde un error que lista A y B, y no lee ni escribe memorias

**Test:** `what_works_on_one_project_asks_for_one_when_several_are_open` (`server.rs`). Hoy no compila: el servidor no conoce las raíces.

## BP9. Una raíz declarada se indexa como el directorio de arranque

El registro de un proyecto MUST mandar `index_docs: true` cuando su raíz es la de una carpeta de lectura que se indexa sola (`auto_indexable`: tiene un `.git` o un manifiesto y no es el directorio personal), sea el directorio de arranque o una raíz del cliente. Cualquier otra raíz, como la de un `project_path` que nombra el modelo, MUST seguir con `false` hasta `clawg projects add`. La decisión MUST ser pura sobre las raíces (`indexes_docs`, `crates/clawg-cli/src/mcp/tools.rs`). El 403 de `search_project` (`not_accepted`, `crates/clawg-cli/src/daemon/api.rs`) MUST decir que el daemon lee una carpeta que el cliente MCP tiene abierta o en la que arrancó un puente, y ya no sólo esta última.

**Tests:** `a_declared_root_is_indexed_like_the_start_directory` (`server.rs`) y `only_a_folder_the_person_opened_is_indexed` (`tools.rs`), que reemplaza a `only_the_bridges_own_root_is_indexed`. Hoy el primero falla: sólo la raíz del directorio de arranque se indexa.

## BP10. La clave de una fila sin proyecto

`search_memory` y `read_memory` MUST mostrar la clave de una `user_preference` sin proyecto como ` topic=K all_projects`, y MUST NOT mostrar la de cualquier otra fila sin proyecto, desde cualquier puente: con proyecto, con un grupo o sin ninguno. Reemplaza «un puente sin proyecto MUST seguir viendo ` topic=K`» de PM8: sin proyecto por defecto para guardar, sólo un save con `all_projects` revisa una fila sin proyecto. Un hit sin `project_id`, de un daemon anterior, MUST mostrar la clave como antes.

**Tests:** `a_hit_without_a_project_shows_how_a_save_revises_it` y `format_save_shows_the_key_only_where_a_save_revises_it` (`tools.rs`), que pasan a pedir lo mismo sin proyecto que con él.

## BP11. Lo que ve el modelo

- La apertura de `protocol.md` MUST pedir que cada save nombre su proyecto con `project_path`, `project` o `all_projects`, en lugar de decir que las memorias son del proyecto actual; la de `protocol.es.md`, lo mismo en español. Cada variante MUST seguir en 2.048 caracteres o menos: sin preferencia 2.030, con `en` 2.047.
- «## Projects» y «## Proyectos» MUST decir que una llamada sin proyecto lee en las carpetas que el cliente tiene abiertas, o en el directorio de arranque; que un save nombra siempre su proyecto; que una búsqueda en varios proyectos marca cada hit con `project=`; y que `recall_context`, `search_project`, `synthesize` y `answer_suggestion` piden uno cuando hay varios.
- Las descripciones de `project_path` de `save_memory`, `search_memory`, `recall_context`, `search_project` y `synthesize`, y la de `project` de `answer_suggestion`, MUST NOT decir que el valor por defecto es el directorio donde arrancó el puente: la de `save_memory` dice que un save nombra su proyecto con ella, con `project` o con `all_projects`, y que sin ninguno se rechaza; las demás, que por defecto son los proyectos de las carpetas abiertas en el cliente, o del directorio de arranque cuando no nombra ninguna, y las de un proyecto, que con varias abiertas se pasa esta o `project`. La de `project` de esas cinco MUST NOT decir «otro que el de este directorio».
- `agent_contract.json` y `agent_contract.es.json` MUST cambiar sólo en las `instructions` y en esas descripciones de `project_path` y `project`, y se bendicen.

**Tests:** `the_protocol_says_where_a_call_without_a_project_goes` (`crates/clawg-cli/src/mcp/protocol.rs`), en los dos idiomas; `the_instructions_carry_the_saving_habits` y `the_spanish_instructions_carry_the_saving_habits`, que pasan a pedir la oración nueva; `the_instructions_fit_older_clients`; `the_spanish_protocol_has_the_sections_and_spans_of_the_english_one`; `the_agent_contract_matches_its_file` y `the_spanish_agent_contract_matches_its_file`, bendecidos. Hoy el primero falla: «## Projects» dice que las memorias son del directorio en el que se trabaja.
