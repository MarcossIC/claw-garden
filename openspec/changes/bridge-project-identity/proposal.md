# bridge-project-identity — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 2 y 4 | M | — | ver [`state.yaml`](state.yaml) |

## Problema

Detalle y evidencia en la [exploración](exploration.md), verificado otra vez contra `develop` (`27c4a88e`).

- **El puente decide una vez, con el directorio donde arrancó.** `ClawgMcp::pending` fija `default_project` con `std::env::current_dir()` (`crates/clawg-cli/src/mcp/server.rs:535`), y cada herramienta sin `project` ni `project_path` lo usa (`resolve_project`, `crates/clawg-cli/src/mcp/tools.rs:1026`). Cursor arranca el proceso en el directorio personal del usuario en los 22 arranques medidos, así que todo lo que guarda o busca sin proyecto va a un proyecto «directorio personal».
- **No pide las raíces del cliente.** No hay `roots/list` en `crates/clawg-cli/src`. Cursor las da: una por carpeta de cada ventana abierta, también en un workspace de varios repositorios, y sin avisar cuando cambian (`roots.listChanged: false`).
- **Un save cae en un proyecto adivinado.** Sin `project` ni `project_path`, `save_memory_impl` guarda en el proyecto del directorio (`server.rs:1083-1090`). Pasa también en Claude Code, donde una conversación puede tratar de otro proyecto que el de su directorio.
- **Una búsqueda va a un proyecto o a todos.** `POST /v1/search` toma un `project_id` (`SearchRequest`, `crates/clawg-cli/src/daemon/api.rs:422`) y el core un `SearchScope.project_id` (`crates/clawg-core/src/memory/search.rs:26`): buscar en un grupo de proyectos no existe.
- **Sólo la raíz del puente se indexa sola.** `index_docs` va sólo para la raíz que resolvió el directorio de arranque (`own_root_to_index`, `server.rs:516`; `indexes_docs`, `tools.rs:1045`). En Cursor esa raíz es el directorio personal, que nunca se indexa, así que `search_project` no tiene qué leer.

## Objetivo que sirve

2, porque una memoria de un repositorio tiene que quedar en ese repositorio y no en otro; 4, porque Cursor es uno de los dos clientes de uso diario. Va en la noche 1 del [ROADMAP](../../ROADMAP.md), antes del piloto que conecta Cursor con `clawg setup cursor`.

## Decisiones

Las decisiones están cerradas en la exploración ([«Decisiones»](exploration.md#decisiones)): B para guardar y A para leer; todo save sin proyecto se rechaza; `search_memory` con varias raíces busca en todas, y cada hit dice su proyecto. Esta propuesta no las reabre: dice cómo se aplican y cierra los tres puntos de [«Para la propuesta»](exploration.md#para-la-propuesta).

1. **Las raíces de cada llamada.** En cada llamada a una herramienta, si el cliente declaró la capacidad `roots` (en `initialize`, o en el `_meta` de la llamada en el ciclo `2026-07-28`; `RequestContext::client_capabilities`), el puente le pide `roots/list` y espera la respuesta hasta `ROOTS_WAIT` (3 s). Una respuesta que falla o no llega cuenta como un cliente sin roots, con un warning en el log. A un cliente que no declaró la capacidad no se le pide. No escucha `notifications/roots/list_changed`: Cursor no la manda, y pedirlas en cada llamada ya da la lista actual.
2. **De una raíz a una carpeta.** Una raíz `file:` se lee como ruta: sin `file://` ni el host `localhost`, con los `%XX` decodificados, y en Windows `/c:/…` queda `c:\…` y otro host es una ruta UNC. Una raíz que no es una URI, como las de Cursor (`c:\…`), se usa como ruta. Una relativa o de otro esquema se descarta. Cada carpeta resuelve con `ProjectIdentity::resolve`, como un `project_path`, así que `c:\x` y `file:///C:/x` llevan a la misma raíz: la canonicalización pone la caja real de la unidad. La lectura de la URI es pura y se prueba sin filesystem.
3. **El directorio personal nunca es un proyecto.** Ni como directorio de arranque ni como raíz: una carpeta cuya ruta o cuya raíz es el directorio personal, canónico, se descarta. Una carpeta dentro de él, con o sin `.git`, sigue siendo un proyecto, como hoy.
4. **Dónde lee una llamada sin proyecto.** En las **carpetas de lectura**: las raíces que dio el cliente, si dio alguna que quede; si no, el directorio donde arrancó el puente, salvo el personal; si no, ninguna. Cada una se registra como hoy (`ids_for`, una vez por raíz y camino en el proceso), y dos que llevan al mismo proyecto, como dos clones de un remoto, cuentan una vez. Sin ninguna, lo que lee va sin proyecto, como hoy un puente cuyo `current_dir()` falló.
5. **Un save dice siempre su proyecto.** `save_memory` sin `project_path`, `project` ni `all_projects` se niega sin llamar al daemon, con una frase en el idioma de la sesión que dice qué pasar y lista las carpetas de lectura como opciones de `project_path`; sin carpetas, sólo los argumentos. No hay más proyecto por defecto para guardar, con ningún cliente.
6. **`search_memory` en un grupo.** Con dos o más proyectos de lectura, `search_memory` busca en el grupo: `POST /v1/search` toma `project_ids`, y el daemon corre la búsqueda de un proyecto en cada uno con el mismo embedding de la consulta, junta los hits por puntaje, deja uno por fila y uno por save, corta en `limit` y aplica `min_relative_score` antes de registrar los accesos. El puntaje de un hit es el de su fila (`score.rs`), el mismo en las dos búsquedas, así que se comparan. La búsqueda de un proyecto no cambia. El puente manda también `project_id` con el primero del grupo, así un daemon anterior, que ignora `project_ids`, busca en un proyecto y nunca en todos.
7. **Cada hit de un grupo dice su proyecto.** En una búsqueda en grupo, la línea lleva `project=<id>` después de `score=`, y la clave de la fila, si tiene, porque un save con ese `project` y esa clave la revisa. Una fila sin proyecto no lleva `project=` y su clave sigue la regla de PM8. La línea de una búsqueda en un proyecto, o en todos, queda byte a byte como hoy.
8. **Las otras herramientas que leen sin proyecto.** `recall_context`, `synthesize`, `search_project` y `answer_suggestion` son de un proyecto: el contexto lleva el workspace, la última sesión y las sugerencias de uno; `synthesize` marca revisadas las memorias que muestra; `search_project` lee el índice de un checkout; `answer_suggestion` responde por uno y puede unir dos. Con un proyecto de lectura usan ese; con varios se niegan, sin buscar ni escribir, con las carpetas de lectura como opciones de `project_path`, como un save; sin ninguno, hacen lo de hoy. Leer de un grupo es de `search_memory`, la herramienta de las preguntas puntuales, que no necesita adivinar el repositorio porque juntar hits no mezcla nada: un contexto o una revisión de un grupo necesitaría un workspace y una última sesión del grupo, que no existen. `memory_status` cuenta los conflictos del proyecto de lectura, o de todos cuando no hay uno. La clave que muestran `read_memory` y una búsqueda con `all_projects` se compara con ese mismo proyecto.
9. **El índice de una raíz.** Una raíz que declaró el cliente es una carpeta que la persona abrió, como el directorio de arranque: se registra con `index_docs` si es un proyecto que se indexa solo (`auto_indexable`: tiene un `.git` o un manifiesto y no es el directorio personal). Una carpeta que el modelo nombra en `project_path` sigue sin indexarse hasta `clawg projects add`.
10. **La clave de una fila sin proyecto.** Como ningún save se guarda sin proyecto por omisión, ningún save común revisa una fila sin proyecto: desde cualquier puente, con proyecto o sin él, una `user_preference` sin proyecto muestra `topic=K all_projects` y cualquier otra fila sin proyecto no muestra clave. Cambia la última frase de PM8 en [`specs/mcp`](../../specs/mcp/spec.md), que dejaba ver `topic=K` desde un puente sin proyecto.
11. **Lo que ve el modelo.** La apertura del protocolo cambia una oración: «Memories are scoped to the current project unless you say otherwise.» pasa a «Name each save's project: `project_path`, `project` or `all_projects`.», y en español «Las memorias son del proyecto actual salvo que digas otra cosa.» pasa a «Nombra el proyecto de cada save: `project_path`, `project` o `all_projects`.». Son 2 caracteres más en inglés, que con `en` deja la apertura en 2.047 de 2.048. «## Projects» y «## Proyectos» dicen dónde lee una llamada sin proyecto, que una búsqueda en varios proyectos marca cada hit con `project=`, y qué herramientas piden uno cuando hay varios. Las descripciones de `project_path` y de `project` dejan de decir «el directorio en el que arrancó este puente». Los dos contratos del agente se bendicen con `CLAWG_BLESS=1`.

## Alcance

1. `crates/clawg-cli/src/mcp/server.rs`: las raíces de la llamada (`roots/list`), las carpetas de lectura, el save que exige su proyecto, la búsqueda en grupo, la negativa con varias de las otras herramientas, `index_docs` de una raíz y las descripciones.
2. `crates/clawg-cli/src/mcp/roots.rs`, nuevo: de una raíz a una ruta, y el directorio personal.
3. `crates/clawg-cli/src/mcp/mod.rs`: el directorio de arranque y el personal llegan al servidor como argumentos, para que un test los elija.
4. `crates/clawg-cli/src/mcp/tools.rs`: `project=` en un hit de un grupo, la regla de la clave de una fila sin proyecto e `indexes_docs` con varias carpetas.
5. `crates/clawg-cli/src/mcp/tools_es.rs`, `protocol.md` y `protocol.es.md`; `agent_contract.json` y `agent_contract.es.json`, bendecidos.
6. `crates/clawg-cli/src/client.rs`: `SearchArgs.project_ids`. `crates/clawg-cli/src/daemon/api.rs`: `project_ids` en `POST /v1/search`.
7. `crates/clawg-core/src/memory/search.rs`: la búsqueda en un grupo (`search_projects`).
8. Los tests que guardaban sin proyecto pasan el suyo: `server.rs`, `crates/clawg-cli/tests/binary.rs`.
9. Docs: `docs/reference/daemon.md` y `docs/reference/data-profiles.md`. `specs/mcp`, `specs/project` y `specs/project-docs` reciben los requisitos de la [spec](spec.md) al archivar.
10. Lo que el CI del pull request marque en Windows.

## Fuera de alcance

- Qué preferencias entran solas a una sesión, y [preference-model](../archive/2026-11-06-preference-model/proposal.md), que se cierra aparte.
- Escuchar `notifications/roots/list_changed`, o guardar las raíces entre llamadas.
- Un contexto, una síntesis, un índice de documentos o una respuesta a una sugerencia sobre un grupo de proyectos.
- Marcar el proyecto en una búsqueda sin proyecto o con `all_projects`, y en `recall_context`.
- La línea del otro idioma en una búsqueda en grupo: el daemon no cuenta `other_language` para un grupo.
- Dejar de pedir las raíces a un cliente que una vez no las dio a tiempo: espera a que aparezca uno.
- Los hooks, que siguen leyendo el `cwd` de su entrada.
- Un `.cursor/mcp.json` por repositorio (enfoque C, descartado en la exploración).
- Mover las memorias que ya cayeron en el proyecto del directorio personal: `update_memory` con `to_project_path` y `clawg projects move` ya lo hacen.
- Subir la versión y archivar.

## Cómo se mide el éxito

Primero, tests que fallan hoy. Corren con el embedder falso y, los de las raíces, con un cliente MCP de prueba que habla por el stdio del puente y contesta `roots/list`:

- Un save sin `project`, `project_path` ni `all_projects` se rechaza con las raíces como opciones; sin roots, con el directorio de arranque como opción; y no se guarda nada.
- Una búsqueda sin proyecto con una raíz busca en su proyecto; con dos, en los dos y no en un tercero, cada hit con su `project=`; sin roots, en el directorio de arranque.
- El directorio personal nunca es un proyecto: ni como directorio de arranque ni como raíz se registra, y un save desde ahí no lo ofrece como opción.
- Una raíz como ruta y como URI `file:` dan el mismo proyecto; en Windows, también con la unidad en minúscula.
- Un cambio de raíces entre dos llamadas se ve en la segunda.
- `recall_context`, `synthesize`, `search_project` y `answer_suggestion` sin proyecto, con dos proyectos de lectura, se niegan nombrando las dos carpetas.
- Una raíz que es un repositorio se registra con `index_docs`, y `search_project` lee su documentación.
- `POST /v1/search` con `project_ids` trae lo de cada proyecto del grupo, una fila sin proyecto una sola vez, y nada de un proyecto de afuera.

Lo que no puede cambiar:

- La búsqueda de un proyecto o de todos: su consulta, sus hits y su línea, byte a byte.
- Un cliente que no declara roots no recibe `roots/list`.
- La suite pasa, y `cargo clippy --workspace --all-targets -- -D warnings` sale con 0 aquí y, por el CI del pull request, en Linux y en Windows.
- El contrato del agente cambia sólo en la apertura, en las descripciones de `project_path` y `project`, y en los dos idiomas.

`rag-eval --diff` no aplica: el ranking de un proyecto no cambia, y el agent view busca con un `project_id`.

Pendiente en local, con Cursor real ([verify](verify.md)): un save desde una ventana con dos repositorios cae en el proyecto que nombró el agente, y una búsqueda con dos ventanas abiertas trae memorias de los dos.

## Datos y vuelta atrás

No toca el esquema ni reescribe filas. `project_ids` es un campo opcional de `POST /v1/search`. Un puente anterior contra este daemon no lo manda y busca como siempre; este puente contra un daemon anterior manda también `project_id` con el primer proyecto del grupo, y ese daemon busca sólo en él: una búsqueda más angosta, nunca más ancha. La negativa del save es del puente, así que un puente anterior sigue guardando en su directorio. Lo que ya se guardó en el proyecto del directorio personal queda donde está, y se mueve con `update_memory`. Para volver atrás alcanza con el binario anterior.
