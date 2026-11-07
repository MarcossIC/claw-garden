# bridge-project-identity — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 2 y 4 | M | — | ver [`state.yaml`](state.yaml) |

## Problema

Detalle, la prueba con Cursor y las decisiones en la [exploración](exploration.md); verificado otra vez contra `develop` (`27c4a88`):

- **El puente decide el proyecto una sola vez, con su directorio.** `ClawgMcp::pending` toma `std::env::current_dir()` al crearse (`crates/clawg-cli/src/mcp/server.rs:535`) y lo guarda en `default_project`. Cada herramienta sin `project` ni `project_path` usa ese proyecto (`resolve_project`, `crates/clawg-cli/src/mcp/tools.rs:1026-1039`), también `save_memory` (`server.rs:1089`).
- **En Cursor ese directorio es el personal del usuario** en los 22 arranques de la prueba, con cualquier combinación de ventanas. Hoy el puente registra esa carpeta como un proyecto y guarda ahí cada memoria de Cursor que no nombra otro.
- **El puente no pide raíces.** No hay `roots/list` en `crates/clawg-cli/src`. Cursor sí las da: una ventana con un repositorio responde una raíz; dos ventanas, las carpetas de las dos, y siempre la lista actual, sin `notifications/roots/list_changed` (declara `roots.listChanged: false`). Cada raíz llega como ruta de Windows con la unidad en minúscula (`c:\...`), no como URI `file:`.
- **Una búsqueda va a un proyecto o a todos.** El predicado de proyecto compara con un solo id (`memory_project_predicate!`, `crates/clawg-core/src/memory/search.rs:110`), y `POST /v1/search` toma un solo `project_id` (`SearchRequest`, `crates/clawg-cli/src/daemon/api.rs:422`). Buscar en las carpetas de un workspace de varios repositorios no se puede.
- **Los documentos de un proyecto sólo se indexan solos si es el directorio del puente** (`own_root_to_index`, `server.rs:516`; `indexes_docs`, `tools.rs:1045`). En Cursor, ningún repositorio abierto lo es.

## Objetivo que sirve

2, porque una memoria tiene que caer en su proyecto, y 4, porque Cursor es el segundo cliente de la meta de esta etapa ([ROADMAP](../../../ROADMAP.md)): sin esto, lo que guarda cae en un proyecto que no existe y lo que busca no ve el de la ventana.

## Decisiones

Las tres de la exploración, cerradas allí, no se reabren: **B para guardar y A para leer**, **todo save sin proyecto se rechaza** y **`search_memory` con varias raíces busca en todas**. Lo que sigue dice cómo se aplican y responde la sección «Para la propuesta».

1. **Las raíces se piden en cada llamada.** Cuando el cliente declara `roots` (en `initialize`, o en el `_meta` de la request en el ciclo 2026-07-28), cada llamada de una herramienta que usa un proyecto por defecto manda `roots/list` y espera la respuesta como mucho 2 s; también la que nombra su proyecto, porque la primera vez que se registra una carpeta abierta se registra con su índice de documentos (decisión 9). No se guarda para la siguiente: Cursor no avisa cuando cambian. Un cliente que no declara `roots` no recibe el pedido. Un error, un plazo vencido o una respuesta sin carpetas cuentan como un cliente sin raíces. `rmcp` marca `list_roots` como obsoleto (SEP-2577, `rmcp-3.5.1/src/service/server.rs:939-943`); el puente lo usa con un `#[expect(deprecated)]` que dice por qué: es lo único que dice qué carpetas tiene abiertas Cursor.
2. **Una raíz es una carpeta.** Un `uri` `file:` se lee como camino, con los `%XX` decodificados, el host vacío o `localhost`, y en Windows la unidad sin la barra de adelante (`file:///c:/x` es `c:/x`). Cualquier otro texto se toma como camino si es absoluto en esta máquina, que es lo que manda Cursor; si no, se ignora. Después, cada carpeta se resuelve como un `project_path` (`ProjectIdentity::resolve`): la misma carpeta escrita de las dos formas da el mismo proyecto.
3. **Las carpetas abiertas.** Son las raíces del cliente, resueltas, sin repetir la misma raíz y sin el directorio personal. Si no queda ninguna, el directorio donde arrancó el puente, salvo que su raíz sea el directorio personal. **El directorio personal nunca es un proyecto**: ni como raíz ni como directorio. Puede no haber ninguna carpeta abierta.
4. **Un save nombra siempre su proyecto,** con cualquier cliente: `project_path`, `project` o `all_projects`. Sin ninguno, `save_memory` se niega sin llamar al daemon, y la frase lista las carpetas abiertas como opciones de `project_path`, en el idioma de la sesión. Sin carpetas abiertas, dice que pase la carpeta del proyecto. El resto de `save_memory` no cambia.
5. **`search_memory` sin `project` ni `project_path` lee las carpetas abiertas.** Ninguna: todos los proyectos, como hoy sin proyecto. Una: su proyecto, como hoy el del directorio. Varias: **el grupo**, cada proyecto con la regla de hoy (sus filas, las sin proyecto y toda `user_preference`). Con varias carpetas abiertas y sin `project` ni `project_path` (un grupo, o todos con `all_projects`), cada hit dice su proyecto, ` project=<id>` después del puntaje, cuando su fila tiene uno: es el id que `project` acepta para guardar ahí. Con una carpeta o ninguna, o nombrando el proyecto, cada hit se muestra como hoy.
6. **El grupo es del core.** El predicado de proyecto pasa a atar una lista JSON de ids en lugar de un id (`m.project_id IN (SELECT value FROM json_each(?))`), con la misma regla, y un proyecto es una lista de uno. `SearchScope` guarda la lista (`with_project` sigue igual y suma `with_projects`), y lo usan todas las consultas que hoy usan el predicado, así la regla vive en un solo lugar. `POST /v1/search` toma `project_ids`, que no va con `project_id`, y cada id pasa por `live_project` como el de hoy. El conteo del otro idioma de esa búsqueda cuenta el grupo.
7. **La clave de un hit en un grupo.** `topic=K` se muestra donde un save la revisaría: con varias carpetas, en una fila de cualquier proyecto del grupo, porque el hit dice su proyecto y un save con ese `project` la revisa. La regla de hoy (`revised_from`, `tools.rs:178`) compara con un proyecto; pasa a comparar con los proyectos de las carpetas, y con uno solo da lo mismo que hoy. Una fila sin proyecto ya no muestra la clave cuando no hay proyecto, porque un save nombra el suyo y sólo uno con `all_projects` la revisa: una preferencia sin proyecto sigue mostrando `all_projects` (PM8 de [`specs/mcp`](../../../specs/mcp/spec.md)), ahora desde cualquier puente, y cualquier otra, ninguna clave (BI5).
8. **Las otras herramientas que leen** (lo que la exploración deja para la propuesta). Lo coherente con `search_memory` es usar las carpetas abiertas, y así lo hacen todas; difieren sólo con varias carpetas, cuando la respuesta es de un proyecto y un grupo no la tiene:

   | Herramienta | Con varias carpetas y sin `project` ni `project_path` | Por qué |
   |---|---|---|
   | `recall_context` | Se niega, lista las carpetas y dice que `search_memory` busca en todas | Su respuesta es de un proyecto: el workspace de ese proyecto, su LAST SESSION y sus sugerencias. Armarla para un grupo cambia qué preferencias entran solas, que queda fuera |
   | `synthesize` sin `all_projects` | Igual que `recall_context` | Marca revisado un proyecto (`synthesis_review`, `crates/clawg-core/src/memory/synthesis.rs:146-150`); un grupo no tiene marca |
   | `search_project` | Igual | El índice es de una carpeta y rankea sus pasajes contra sí mismos |
   | `answer_suggestion` | Igual | Es una respuesta de la persona sobre un proyecto |
   | `memory_status` | Cuenta los pares de todos los proyectos | No toma argumentos, y no puede fallar por una línea |
   | `read_memory` | Muestra la clave de una fila de cualquiera de los proyectos | La misma regla que `search_memory` (decisión 7) |

   Con una carpeta, cada una hace lo de hoy con el directorio; sin ninguna, lo de hoy sin proyecto. `list_memories`, `delete_memory`, `restore_memory`, `update_memory` y `resolve_conflict` no usan un proyecto por defecto y no piden raíces.
9. **Los documentos de las carpetas abiertas.** Una carpeta abierta se registra con `index_docs` si tiene `.git` o un manifiesto y no es el directorio personal (`auto_indexable`, `crates/clawg-core/src/memory/project_docs/corpus.rs:26`), como hoy el directorio del puente: es una carpeta que la persona abrió. Cualquier otro camino que nombre el modelo sigue siendo una etiqueta hasta `clawg projects add`.
10. **Lo que ve el modelo.** Las descripciones de `project_path` y `project` de `save_memory` dicen que nombrar uno de los tres es obligatorio; las de las herramientas que leen, que sin él leen las carpetas que el cliente tiene abiertas, o el directorio, y qué hacen con varias. La de `search_memory` dice que con varias carpetas busca en todas y que cada hit nombra su proyecto. La apertura del protocolo y su sección «Projects», en los dos idiomas: un save nombra su proyecto, y con varias carpetas se pasa `project_path`. `agent_contract.json` y `agent_contract.es.json` se bendicen con `CLAWG_BLESS=1`, y el diff va en el pull request.

## Alcance

1. `crates/clawg-cli/src/mcp/roots.rs`, nuevo: leer una raíz, las carpetas abiertas (puro, con el resolvedor como argumento) y el pedido de `roots/list` con su plazo.
2. `crates/clawg-cli/src/mcp/server.rs`: las raíces de cada llamada en un clon del servidor, el save sin proyecto, el grupo de `search_memory`, las herramientas de la decisión 8, el directorio personal y el índice de documentos. `tools.rs` y `tools_es.rs`: las frases, la marca de proyecto y la clave contra varios proyectos.
3. `crates/clawg-core/src/memory/search.rs`: el predicado con una lista y `SearchScope`; `language.rs`, `dispute.rs`, `crowding.rs`, `prompt_builder.rs` y `eval_api.rs` atan la lista. `crates/clawg-cli/src/daemon/api.rs` y `client.rs`: `project_ids` en la búsqueda.
4. `protocol.md`, `protocol.es.md` y los dos contratos, bendecidos.
5. Docs: `docs/reference/daemon.md` (herramientas MCP y la búsqueda) y `docs/reference/data-profiles.md` (de dónde sale el proyecto del puente).
6. `openspec`: esta propuesta, la [spec](spec.md), las [tareas](tasks.md), `verify.md`, `state.yaml` y la línea de `ROADMAP.md`. Los requisitos van a `specs/mcp` y `specs/retrieval` al archivar, que queda fuera.
7. Lo que el CI del pull request marque en Windows.

## Fuera de alcance

- Qué preferencias entran solas a una sesión: el workspace, su consulta y el contexto de un grupo. Por eso `recall_context` no arma un grupo (decisión 8).
- El hook `session-start`: con Cursor ya lee `workspace_roots` y no adivina con varias ([`specs/hooks`](../../../specs/hooks/spec.md), CU7 de client-setup-cursor); con Claude Code sigue tomando su `cwd`, también si es el directorio personal.
- `notifications/roots/list_changed`: Cursor no la manda, y pedir las raíces en cada llamada la hace innecesaria.
- Un `.cursor/mcp.json` por repositorio (enfoque C, descartado en la exploración).
- Mover las memorias que ya están en el proyecto del directorio personal: `update_memory` con `to_project_path` lo hace fila por fila, y `clawg projects` mueve un proyecto entero.
- [preference-model](../2026-11-06-preference-model/proposal.md), que se cierra aparte; subir la versión y archivar.

## Cómo se mide el éxito

Primero, tests que fallan hoy, con un cliente MCP de prueba que habla por un `duplex` como un host real y declara `roots` o no, en Linux con el embedder falso. Los nombres, en la [spec](spec.md):

- un save sin `project`, `project_path` ni `all_projects` se rechaza con las raíces como opciones, con roots y sin roots;
- una búsqueda sin proyecto con una raíz busca en su proyecto, con varias en todas y nombra el proyecto de cada hit, y sin roots usa el directorio;
- el directorio personal nunca es un proyecto: ni como raíz ni como directorio del puente;
- una raíz como ruta y como URI `file:` dan el mismo proyecto;
- un cambio de raíces entre dos llamadas se ve en la segunda, sin `list_changed`;
- un cliente sin la capacidad no recibe ningún `roots/list` y sigue como hoy, salvo el save sin proyecto.

Lo que no puede cambiar:

- Una búsqueda de un proyecto, o de todos, devuelve las mismas filas y la misma línea de hit que hoy; los tests de búsqueda, de contexto, de pares y de idioma del core pasan sin cambiar lo que esperan.
- La suite pasa, y `cargo clippy --workspace --all-targets -- -D warnings` sale con 0 en Linux y, por el CI del pull request, en Windows.
- El contrato cambia sólo en las descripciones de la decisión 10, en los dos idiomas.

`rag-eval --diff` no aplica: el ranking de un proyecto no cambia, y `rag-eval` busca sin proyecto, que tampoco cambia.

Pendiente en local, con Cursor real ([verify](verify.md)): un save desde una ventana con dos repositorios cae en el proyecto que nombró el agente, y una búsqueda con dos ventanas abiertas trae memorias de los dos.

## Datos y vuelta atrás

No toca el esquema, los identificadores guardados, los estados ni el contrato de embedding, y no reescribe ninguna fila. Las memorias que Cursor ya guardó en el proyecto del directorio personal quedan ahí: se siguen encontrando con `all_projects` y se mueven con `update_memory`. Un binario anterior abre la base sin cambios y vuelve a la conducta de antes.

Un puente anterior contra este daemon no manda `project_ids` y busca como siempre. Este puente contra un daemon anterior manda `project_ids`, que ese daemon ignora, y también el primer proyecto del grupo en `project_id`, así la búsqueda de un grupo sale en ese proyecto: más angosta, nunca en todos; se arregla reiniciando el daemon con el binario nuevo. Para volver atrás alcanza con el binario anterior.
