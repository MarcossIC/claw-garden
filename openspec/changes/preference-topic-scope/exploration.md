# preference-topic-scope — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | S | topic-revisions (27, archivado); antes de 44 | exploración (borrador) |

**Tesis:** `search_memory` muestra ` topic=K` en una fila propia de otro proyecto y `protocol.md` le dice al agente que guarde la versión nueva «bajo esa clave, la que se muestra». Pero una clave revisa sólo dentro de su proyecto, así que el save no reemplaza nada y deja dos filas vivas con la misma clave. La respuesta del save no avisa. Lo más chico que cierra el engaño es que el puente deje de mostrar la clave donde no vale.

## Lo que hay hoy (verificado contra el código)

- **La búsqueda cruza proyectos.** `memory_project_predicate` lee las filas del proyecto, las sin proyecto y **toda** `user_preference` (`crates/clawg-core/src/memory/search.rs:92-99`; la doc dice que sigue a la persona de un proyecto a otro, `:87-91`). Cada `RankedHit` lleva `topic_key` (`search.rs:129`) pero **no** `project_id`; el `Hit` del cliente tampoco (`crates/clawg-cli/src/client.rs:166-192`).
- **El puente muestra la clave.** `format_hits` la imprime en todo hit del lector y la oculta sólo en los de otro agente (`crates/clawg-cli/src/mcp/tools.rs:111-113`). `read_memory` (`SavedText`, `client.rs:494-510`, `tools.rs:183`), `restore_memory` (`tools.rs:231`) y `list_memories` (`tools.rs:294-302`) también la muestran. Ninguno dice el proyecto de la fila, aunque `MemoryRow` sí lo trae (`client.rs:224`). `recall_context` y `session-start` no muestran claves.
- **El alcance de la revisión incluye el proyecto.** `SCOPE_ROWS_SQL` compara agente, `COALESCE(project_id, '')` y clave (`crates/clawg-core/src/memory/topic.rs:83-86`); `specs/ingest/spec.md:30` lo fija como requisito («otro agente u otro proyecto con la misma clave no se tocan»). El save toma el proyecto del puente o de su `project_path` (`crates/clawg-cli/src/mcp/server.rs:746`), no el de la fila que el agente vio.
- **El protocolo promete la revisión.** `crates/clawg-cli/src/mcp/protocol.md:73-74`: guardá la versión nueva bajo la clave mostrada, «never a new one».
- **La respuesta no avisa.** Sin filas reemplazadas el texto es sólo `saved #N` (`server.rs:600`); el único aviso de «no reemplazó nada» es `key_dropped`, para un daemon viejo (`server.rs:614-619`, `client.rs:920`).
- **Alcance más ancho que las preferencias.** El predicado también lee las filas **sin proyecto** (`project_id IS NULL`), y su clave se muestra igual; un save desde un proyecto tampoco las revisa (`COALESCE` `''` contra el id del proyecto).
- **Los otros caminos comparten el alcance y están bien así:** el revival (`revival.rs:66-80`), el `restore` (`KEY_HELD_SQL`) y el import (`TOPIC_HOLDERS_SQL`, `portability/import.rs:247-250`) comparan agente y proyecto de la fila; no hay desajuste ahí, porque parten de la fila y no de lo que el agente vio. Cambiar el alcance (B) los toca a todos.

## Alternativas

| | Qué ve el agente | Datos | Costo | Interacciones |
|---|---|---|---|---|
| **A. El puente oculta ` topic=K` si el proyecto de la fila no es el del save** | La clave sólo donde revisa | Ninguno | Sumar `project_id` a `RankedHit` y `Hit` (ya está en `m.*`, `search.rs:242`, `:1035`) y comparar en `format_hits`. Cero tokens extra, en realidad menos. Un daemon viejo no manda el campo: se muestra como hoy | Prueba que falla hoy: un hit propio de otro proyecto con clave no debe mostrarla (junto a `tools.rs:685-705`). No cambia ranking ni el alcance de 27. 44 importa con claves en el proyecto de cada nota, así que no depende. Falla: el puente compara contra el proyecto de la búsqueda, y un save con otro `project_path` revisa otra cosa |
| **B. Un save bajo una clave que tiene una `user_preference` viva del agente en otro proyecto la revisa** | Lo que promete el protocolo | Deja `superseded` una fila de otro proyecto: no se pierde, pero **desaparece de ese proyecto**. Dos proyectos con preferencias distintas bajo una clave (`style/indent`) se pisarían sin que nadie lo pida | Cambia `specs/ingest:30`, `SCOPE_ROWS_SQL`, revival, `restore` e import; el tipo de la versión nueva puede heredarse, así que el save no sabe en el momento si es una preferencia. Mucho más grande | Rompe la regla «ningún cambio oculta memorias». Con 44 una importación podría pisar preferencias de otro proyecto en volumen |
| **C. La respuesta del save dice que no reemplazó nada mientras una fila viva del agente en otro proyecto tiene esa clave** | Se entera después de guardar | Ninguno | Una consulta por `idx_memory_topic` (agente primero) en la transacción; un campo nuevo en la respuesta y texto en `server.rs:610`; unos 25 tokens sólo cuando pasa | No evita las dos filas vivas, sólo avisa. Con 44 daría un aviso por nota importada |
| **D. Mostrar la clave junto al proyecto (` topic=K project=P`)** | Todo y decide él | Ninguno | Sumar tokens en cada hit con clave, y el agente no tiene cómo pasar un `project_id` (`project_path` es una ruta; el hit trae un id) | No sirve sin una ruta que el daemon no devuelve |

## Recomendación

**A.** Cierra el engaño en su origen con el cambio más chico: la herramienta deja de prometer una revisión que el save no hace. No toca datos, esquema, alcance ni ranking, y el costo en tokens es negativo. Si un agente en B quiere cambiar la preferencia de A, la guarda de nuevo: el detector empareja las dos preferencias cercanas y las escala (`ESCALATE_BELOW` 0,40, `contradiction.rs:63`), que es el camino seguro y ya existe. B se descarta por esconder una fila de otro proyecto. C puede sumarse después si se mide que los agentes inventan la misma clave sin verla.

## Decisiones abiertas

Son del usuario.

- [ ] **Qué proyecto compara A.** El de la búsqueda (el puente o `project_path`; recomendado) o el del puente aunque la búsqueda diga otro.
- [ ] **Superficies.** Sólo `search_memory` (recomendado, es donde el protocolo manda reusar la clave) o también `read_memory`, `list_memories` y `restore_memory`, que hoy no dicen el proyecto de la fila.
- [ ] **Filas sin proyecto.** Tratarlas como de otro proyecto (recomendado: es lo que hace el alcance) o dejarles la clave.
- [ ] **Protocolo.** Sumar a `protocol.md:73-74` que una clave sólo se muestra donde revisa.
- [ ] **C.** Dejarlo afuera (recomendado) o sumarlo.

## Cómo se mide

Con el embedder falso. RED: una prueba de `format_hits` con un hit propio de otro proyecto y clave que hoy la imprime; y una de punta a punta con el daemon que guarda una preferencia bajo K en A, la busca desde B, guarda bajo K y comprueba que hay dos filas vivas (hoy sí, la prueba fija el motivo). Sin `rag-eval`: el ranking no cambia, y `revision-eval` (48) no se mueve porque sus casos tienen un solo proyecto.

## Riesgos

- El puente decide con lo que sabe de la búsqueda, no del save que vendrá; un `project_path` distinto en el save lo desmiente.
- Un daemon anterior no manda `project_id`: se muestra la clave como hoy, sin regresión.
- Ocultar la clave puede aumentar los saves sin clave de preferencias de otros proyectos y, con ellos, pares que juzgar.
