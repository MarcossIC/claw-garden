# agent-authorship — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 2, 4 | M–L | nada pendiente: los cambios que fijan la regla de hoy (visibility-policy 6, agent-sharing 16, topic-revisions 27, memory-provenance 41, protected-verdicts 63) están archivados. history-import (44) depende de este cambio | propuesta; las seis decisiones abiertas se cerraron con la opción recomendada ([propuesta](proposal.md#decisiones)) |

**Tesis:** hoy `agent_id` hace dos trabajos: dice quién escribió una fila y decide qué puede hacer con ella cada agente. El segundo nació cuando nada se compartía y cualquier agente podía borrar una fila ajena con sólo conocer su id (visibility-policy); agent-sharing abrió la lectura por búsqueda y dejó todo lo demás atado al autor. El resultado es un cerebro compartido a medias: un agente encuentra la nota de otro en `search_memory` y no puede leerla entera, corregirla, borrarla ni juzgarla; guarda un texto que otro ya guardó y queda repetido; revisa un tema que otro ya cambió y quedan dos versiones vigentes. El cambio separa los dos trabajos: `agent_id` dice quién escribió una fila y nunca quién puede leerla o cambiarla, y las reglas que comparan un save con lo guardado (dedup, detector de contradicciones, clave de tema, síntesis) se aplican por proyecto y no por agente y proyecto. Lo que describe la actividad propia de un agente (su `workspace.md`, RECENT, la sesión del host que tiene registrada, el aviso del hook) sigue siendo por agente, y el autor se sigue mostrando. La única frontera que queda es la que el usuario escribe a propósito en `sharing.json`. Es de tamaño M–L porque son 36 lugares que hoy filtran por agente o describen ese filtro, un cambio de índices (V20) que no toca ninguna fila, y trece specs. Importa ahora porque history-import (44) trae 3.713 notas bajo el autor `migrated`, el 55 % más largas que una pieza, y hoy el agente del usuario sólo las vería como fragmentos de 500 caracteres que no puede leer enteros ni corregir.

## Premisas que se verificaron

Cada cita se leyó en el código el 2026-10-22. No se construyó ni se corrió nada, y no se abrió ningún perfil de datos.

| Premisa | Resultado |
|---|---|
| `agent_id` decide quién lee entera una fila | **Cierta.** `read_save_core` filtra la fila pedida y cada posición del save comparando `agent == Some(o)` (`crates/clawg-core/src/memory/pieces.rs:107-109`, `:113`, `:157`), y el puente le pasa su agente (`crates/clawg-cli/src/mcp/server.rs:908`). Una fila de otro agente es «no existe»; un pedazo ajeno dentro de un save figura como faltante. |
| `update`, `delete`, `restore` y `judge` rechazan la fila de otro agente | **Cierta.** `UPDATE_READ_*` (`crates/clawg-core/src/memory/admin.rs:213-218`), `OWNED_ELSEWHERE_*` (`admin.rs:308-310`, usado en `:366-370`), `RESTORE_READ` (`admin.rs:460-464`) y `SELECT_PAIR_ROW`, que se lee para las dos filas del par (`crates/clawg-core/src/memory/dispute.rs:222-223`, `:355-356`). Las rutas del daemon pasan `agent` o `agent_id` (`crates/clawg-cli/src/daemon/api.rs:1494`, `:1510`, `:1533`, `:1591`, `:1604-1612`). |
| Una fila sin autor la puede cambiar cualquier agente que la ve | **Falsa.** `agent_id IS NOT ?2` es verdadero con NULL (`admin.rs:308`) y `agent_id = ?2` es falso (`admin.rs:214`, `:464`; `dispute.rs:223`): para un agente que manda su id, una fila sin autor es «no existe», aunque la búsqueda se la muestre (`Reach::admits`, `crates/clawg-core/src/memory/sharing.rs:55-62`). `read_save_core` hace lo mismo (`agent == Some(o)`). |
| Dedup, detector, clave de tema y síntesis comparan sólo con el propio agente | **Cierta.** `REINFORCE_LOOKUP_SQL` (`crates/clawg-core/src/memory/store_insert.rs:300-307`), `Reach::OwnOnly` (`crates/clawg-core/src/memory/contradiction.rs:179-182`), `SCOPE_ROWS_SQL` (`crates/clawg-core/src/memory/topic.rs:83-86`), `PENDING` y el filtro del alcance (`crates/clawg-core/src/memory/synthesis.rs:32-36`, `:141-150`). Tres tests lo fijan: `reinforce_if_duplicate_other_agents_row_is_not_reinforced` (`store_insert.rs:2066`), `a_key_is_scoped_to_its_agent_and_project` (`crates/clawg-core/src/memory/worker.rs:4183`) y `contradiction_probe_ignores_shared_authors` (`contradiction.rs:600`). |
| La búsqueda ya alcanza a los demás agentes | **Cierta.** `reach()` resuelve `sharing.json` en cada pedido (`api.rs:1310-1318`) y entra a cada consulta (`crates/clawg-core/src/memory/search.rs:64-85`). Un hit ajeno se muestra, cuenta como acceso (`api.rs:1299`) y no se puede leer entero. |
| `sharing.json` sólo acota la lectura | **Cierta.** Sólo la búsqueda, el contexto y LAST SESSION usan `Reach` (`api.rs:1276-1278`, `:1352`, `:1361`; `crates/clawg-core/src/memory/prompt_builder.rs:873`). Ningún camino que modifica lo consulta: los núcleos reciben un `Option<&str>` con el agente. |
| El `?agent=` del daemon se puede quitar sin consecuencias | **Falsa.** `OwnerQuery`, `SynthesisRequest` y `DisputesQuery` son `deny_unknown_fields` (`api.rs:456-465`, `:698-710`, `:848-853`): un daemon que dejara de aceptar `agent` contestaría 400 a un puente anterior que lo manda. |
| Las rutas del usuario ya no tienen frontera | **Cierta.** `GET /v1/memories/{kind}/{id}` no mira el agente (`api.rs:1454-1464`); `list` y `stats` filtran en vez de limitar (`admin.rs:804-805`, `:869-872`); `clawg undelete` y `clawg outbox` van sin agente (`crates/clawg-cli/src/undelete.rs:37`, `crates/clawg-cli/src/outbox.rs:32-36`). |
| El texto de un par sólo se imprime si su dueño puede leerlo | **Falsa, sin consecuencia hoy.** `fetch_active_conflicts` trae `m_old.content` y `m_new.content` por el agente de la fila nueva sin mirar `Reach` (`prompt_builder.rs:1068-1081`), y el otro lado puede estar fuera del prompt ([`specs/retrieval`](../../../specs/retrieval/spec.md), «Conflictos por referencia»). Hoy los dos lados son del mismo agente; con pares entre agentes sería una fuga de una fila oculta. |
| Los índices de dedup y de clave sirven una búsqueda por proyecto | **Falsa.** Los dos empiezan por `agent_id` (`idx_memory_dedup`, `crates/clawg-core/migrations/V18__session_indexes.sql:21-24`; `idx_memory_topic`, `crates/clawg-core/migrations/V16__topic_key.sql:13-15`). Sin una igualdad sobre esa columna no hay búsqueda por índice, y el proyecto no usa estadísticas del planificador (query-plans, 56). Se confirma con `EXPLAIN QUERY PLAN`, como hace `reinforce_lookup_uses_dedup_index` (`store_insert.rs:2165-2183`). |
| `synthesize` sólo sabe mirar a un agente | **Falsa.** Sin `agent_id` el pedido es del dueño y ve todo el alcance (`synthesis.rs:9-12`, `:106-113`). Lo que lo ata es que el puente manda el agente (`server.rs:1032`), que `PENDING` compara el agente de la conclusión (`synthesis.rs:35`) y que la revisión se guarda por `(agent_id, scope_key)` (`synthesis.rs:135-137`, `crates/clawg-core/migrations/V10__synthesis.sql:22-27`). |
| Borrar una fila protegida pide la palabra del usuario | **Falsa.** La guarda de 63 cubre reemplazarla con un veredicto (`dispute.rs:365-367`) y sacarle el tipo (`admin.rs:266-269`); borrarla no mira el tipo (`admin.rs:342-397`). Hoy sólo la borra su autor; con el cambio, cualquier agente que la alcance. |
| Las descripciones, el protocolo y el contrato dicen que lo ajeno es de sólo lectura | **Cierta.** `server.rs:1170-1172`, `:1189-1190`, `:1208-1209`, `:1226-1227`; `crates/clawg-cli/src/mcp/agent_contract.json` (líneas 5, 152, 200 y 417); `crates/clawg-cli/src/mcp/protocol.md:158-159`; `SHARED_NOTE` («only that agent can change or delete them», `prompt_builder.rs:35-36`). Un test fija las descripciones (`tool_descriptions_say_shared_memories_are_read_only`, `server.rs:3675`). |
| «Por proyecto» cambia a qué proyectos llega el detector | **Falsa.** El predicado de proyecto ya trae las filas del proyecto, las sin proyecto y toda `user_preference` (`search.rs:87-99`), y `Candidate.same_scope` ya compara sólo el proyecto (`contradiction.rs:84-87`, `:225`). Quitar el filtro de autor no mueve eso. |

## Problema (verificado contra el código)

### Cómo se llegó a la regla de hoy

- **visibility-policy (6)** tenía dos problemas con el agente ([propuesta](../2026-10-12-visibility-policy/proposal.md), líneas 11-12). El primero: la identidad era frágil (salía sólo de `clientInfo.name`, con `mcp` de respaldo), y un cliente que dejara de mandarla perdía sus memorias sin error. El segundo: `list`, `delete` y `resolve` no miraban quién llamaba, así que cualquier agente podía borrar o reemplazar cualquier memoria conociendo su id, en un momento en que la búsqueda tampoco compartía nada. Resolvió el segundo con una frontera por agente en lo que modifica, igual que la de la búsqueda (líneas 37-42), y el primero con `--agent`.
- **agent-sharing (16)** abrió la lectura: por defecto cada agente lee a todos, y el usuario oculta con `sharing.json`. Dejó atado al autor lo que modifica («lo ajeno es de sólo lectura», [propuesta](../2026-10-14-agent-sharing/proposal.md), línea 23) y lo que compara: workspace, ACTIVE CONFLICTS, dedup, detector, `synthesize` y `list_memories` (línea 25). Dejó fuera detectar contradicciones entre agentes (línea 40), y puso la fila ajena de sólo lectura entre las mitigaciones de una memoria envenenada (línea 73).
- **Lo que este cambio hace con eso.** El segundo problema desaparece como regla: ya no hay frontera por agente, y lo que protegía queda cubierto por otras mitigaciones (ver «Riesgos»). El primero se arregla en lo que importaba: cualquier identidad lee entero, cambia, borra y juzga todo lo que alcanza, así que un cliente que cae a `mcp` ya no deja sus filas fuera de alcance. Queda frágil sólo lo que sigue siendo por agente: el `workspace.md`, RECENT y el aviso del hook de quien guarda bajo otro nombre. Por eso el hallazgo `fallback_agent` de `clawg doctor` se queda, con otro texto.

### El inventario

Se buscó cada lugar donde el agente decide qué puede leerse, cambiarse o compararse, o donde un texto dice que lo decide. Son 36: 31 cambian y 5 se quedan por agente a propósito. Los de abajo del todo nombran al agente sin ser una frontera.

**Acceso a una fila por su id**

| # | Lugar | Dónde | Hoy | Con el modelo |
|---|---|---|---|---|
| 1 | Leer entera (`read_memory`) | `pieces.rs:98-113`, `:157`; `api.rs:1466-1479`; `server.rs:899-918` | La fila y cada pedazo de su save deben ser del agente; si no, «no existe» o faltante | Lee lo que alcanza, de cualquier autor y sin autor. Desaparece el hueco por «pedazo ajeno» (`Position`, `pieces.rs:94-96`) |
| 2 | Cambiar importancia o tipo | `admin.rs:213-218`, `:233-239`; `api.rs:1481-1502`; `server.rs:935-961` | `(?2 IS NULL OR agent_id = ?2)` al leer la fila | Sin filtro de autor; lo que el lector alcanza |
| 3 | Borrar | `admin.rs:308-310`, `:342-370`; `api.rs:1504-1518`; `server.rs:865-880` | `OWNED_ELSEWHERE_*` corta antes de escribir y de revivir | Igual que 2. `discard_failed` (`crates/clawg-core/src/runtime.rs:289-298`) manda el agente del save fallido y no cambia: borra lo que ese save insertó |
| 4 | Restaurar la fila | `admin.rs:460-464`, `:619-640`, `:669-672`; `api.rs:1520-1543`; `server.rs:882-897` | `RESTORE_READ` con agente, antes y dentro de la transacción | Igual que 2 |
| 5 | Juzgar un par | `dispute.rs:222-244`, `:303-356`; `api.rs:818-846`, `:1586-1616`; `server.rs:963-988` | Las dos filas deben ser del agente | Cualquier agente juzga un par cuyas dos filas alcanza |
| 6 | `derived_from` | `api.rs:1141-1161`; `synthesis.rs:327-388` | `existing_memory_ids` y `replaced_memory_ids` con el agente: una fuente ajena es «no existe» (400) | Acepta fuentes de cualquier autor que alcanza |
| 7 | Listar | `server.rs:835-863` (`:852`); `tools.rs:301-325`; `admin.rs:799-805` | El puente manda `agent: Some(agent_id)`; la línea no muestra autor | Decisión abierta 4 |
| 8 | Pares abiertos | `dispute.rs:440-445` (`n.agent_id = ?1`); `api.rs:848-853`, `:1619-1627`; `server.rs:1043-1070` (`:1056`); `tools.rs:168-192` | Cuenta los pares cuya fila nueva es del agente | Decisión abierta 3 |

**Con qué filas se compara un save**

| # | Lugar | Dónde | Hoy | Con el modelo |
|---|---|---|---|---|
| 9 | Dedup | `store_insert.rs:300-307` (usada en `:384` y `:457`); `worker.rs:593-601` | `agent_id = ?1` más proyecto y hash: el texto de otro agente se guarda otra vez | Igualdad de proyecto y hash; el save refuerza la fila más vieja del proyecto, sea de quien sea. Índice nuevo (V20) |
| 10 | Gemela viva al restaurar | `admin.rs:468-473` | `t.agent_id IS m.agent_id` | Por proyecto |
| 11 | Detector | `contradiction.rs:172-184` (`:182`), `:186-212`; `worker.rs:911-913`; `sharing.rs:48-50`, `:55-57`; `search.rs:64-68` | `Reach::OwnOnly`: sólo filas del que guarda, ni siquiera las sin autor | Las filas que alcanza el predicado de proyecto de hoy, de cualquier autor que el que guarda alcanza, las sin autor incluidas. `OwnOnly` deja de existir |
| 12 | Detector al restaurar | `admin.rs:510-540` (`:519-521`) | Una fila sin autor no busca candidatas | Busca como cualquier otra |
| 13 | Aviso de revisión sin clave | `contradiction.rs:84-87`, `:225`; `topic.rs:175-184`; `worker.rs:704-705` | `same_scope` ya compara sólo el proyecto, pero las candidatas son todas del mismo agente | Puede nombrar la fila de otro agente |
| 14 | Clave de tema: qué reemplaza | `topic.rs:47-54`, `:81-86`, `:98-104`, `:119-136`; `worker.rs:677-681` | `TopicScope { agent_id, project_id, key }` | `(proyecto, clave)`: un save con clave reemplaza a todo titular vivo del proyecto. Índice nuevo (V20) |
| 15 | Titular de la clave al restaurar | `admin.rs:475-486` | Mismo agente y proyecto | Por proyecto |
| 16 | Revivir al borrar | `revival.rs:64-80` (`:67`, `:78`) | Una fila es «revisión» sólo si la reemplazó un save del mismo agente; `KEY_HELD_SQL` busca titulares del mismo agente | Sin la condición de agente. Si no, una revisión entre agentes se leería como veredicto y borrar la vigente no revivirá a la anterior |
| 17 | Import | `crates/clawg-core/src/memory/portability/import.rs:63-64`, `:236-250`, `:296-308` | `TWIN_SQL` y `TOPIC_HOLDERS_SQL` por agente y proyecto | Por proyecto, como el worker. `PRESENT_SQL` queda: es la identidad de una fila ya importada |
| 18 | Síntesis: pendientes | `synthesis.rs:9-12`, `:32-36` (`:35`), `:141-150` | `PENDING` exige que la conclusión sea del mismo agente; el alcance filtra `agent_id = ?` | Pendiente: viva y sin conclusión viva de cualquier autor. Alcance: el proyecto, que ya es el modo «dueño» (`agent: None`) |
| 19 | Síntesis: revisión y recordatorio | `synthesis.rs:106-113`, `:212-325`; `V10__synthesis.sql:22-27`; `api.rs:698-710`, `:1369-1377`, `:1394-1403`; `server.rs:1019-1041` | `synthesis_review` por `(agent_id, scope_key)`; `unreviewed` y `conclusions_to_review` del agente del pedido | Revisión por proyecto (`agent_id = ''`). Ver «Una base existente» |
| 20 | Doctor: invariantes | `crates/clawg-core/src/memory/inspect.rs:72`, `:78`, `:233-266`, `:291-314`; `crates/clawg-cli/src/doctor/checks.rs:404-419`, `:463-478` | `memory.topic` y `memory.content_hash` agrupan por agente y proyecto | Por proyecto. Decisión abierta 5 |

**Vistas y lo que ve el modelo**

| # | Lugar | Dónde | Hoy | Con el modelo |
|---|---|---|---|---|
| 21 | ACTIVE CONFLICTS | `prompt_builder.rs:337`, `:1055-1110` (`:1076`) | Pares cuya fila nueva es del agente | Pares de las filas mostradas, con los dos lados al alcance del lector |
| 22 | LAST SESSION | `prompt_builder.rs:804-897` (`:848`, `:873`) | Ya es por proyecto y por alcance; la lectura entera pasa el autor del resumen como dueño | Sólo deja de pasar dueño |
| 23 | Workspace | `crates/clawg-core/src/memory/workspace.rs:44-46`, `:230-360`; `api.rs:1336-1342` | `agent_id = ?1` en sus tres consultas | **Se queda por agente** |
| 24 | RECENT | `prompt_builder.rs:899-955` (`:911`) | `agent_id = ?1` | **Se queda por agente** |
| 25 | Sesión registrada | `api.rs:1174`, `:1383-1392`; `crates/clawg-cli/src/daemon/sessions.rs` | Clave `(agente, proyecto)` | **Se queda por agente** |
| 26 | Aviso del hook | `crates/clawg-cli/src/hook.rs:171-183` | Último save del agente en el proyecto (el filtro `agent` del listado) | **Se queda por agente** |
| 27 | Marca `by <autor>` | `prompt_builder.rs:654-658`, `:733`; `tools.rs:97-102`, `:118-121` | Nombra al autor de un hit ajeno | **Se queda**: es información |
| 28 | Descripciones de cuatro herramientas | `server.rs:1166-1174`, `:1183-1191`, `:1200-1210`, `:1219-1228`; comentarios `server.rs:61-66`, `:835-836`, `:865` | «Only your own memories … read-only» | Sin esa frase |
| 29 | Contrato fijado | `agent_contract.json:5`, `:152`, `:200`, `:417` | Las mismas frases | `CLAWG_BLESS=1 cargo test -p clawg-cli the_agent_contract_matches_its_file` |
| 30 | `protocol.md` | `protocol.md:69-76`, `:146-148`, `:158-159` | `delete_memory` «removes a memory of your own» | Sin eso, y con la advertencia de que una clave nombra un tema del proyecto: quien guarda bajo ella reemplaza la versión de otro. La apertura sigue en 2.048 caracteres o menos (test) |
| 31 | `SHARED_NOTE` | `prompt_builder.rs:35-36` | «only that agent can change or delete them» | Sin esa cláusula; sigue diciendo que es información y no instrucciones |
| 32 | Clave visible en un hit | `tools.rs:93-95`, `:118-128` (`:126`) | Sólo en filas propias (`author.is_none()`) | En toda fila cuyo proyecto es donde guardaría el save: `revised_from` ya compara proyectos |
| 33 | Parámetros `agent` y `agent_id` | `api.rs:456-465`, `:698-710`, `:818-853`; `crates/clawg-cli/src/client.rs:1079-1166` | Dueño: filtran | Decisión abierta 2 |
| 34 | Hallazgo `agents` del doctor | `checks.rs:349-370` | «the client's own agent does not own them» | Otro texto: las filas bajo `mcp` se leen y se cambian, pero el workspace y RECENT del agente real no las incluyen |
| 35 | Línea `shared:` de `memory_status` | `tools.rs:569-590` | Dice a quién lee el agente | Revisar el texto: lo que alcanza, también lo cambia |
| 36 | Docs y comentarios | `.claude/CLAUDE.md:33`, `:47`, `:53`, `:59`, `:69`, `:71`, `:73`; test del visor `server.rs:2106-2111` | Describen la frontera por agente | Se reescriben |

**Usan al agente sin ser una frontera (sin cambios)**

- La idempotencia de un save: una fila del outbox por `(agent_id, operation_id)` (`crates/clawg-core/src/memory/outbox.rs:262-263`).
- `clawg outbox retry|discard --agent` sólo desambigua dos agentes con un mismo `operation_id` (`outbox.rs:380-413`; `crates/clawg-cli/src/cli.rs:353-356`).
- `clawg purge` quita las filas del outbox del save de la fila, que son de su agente (`crates/clawg-core/src/memory/secrets.rs:153-195`).
- `clawg export --agent` filtra e `import --agent` reasigna (`cli.rs:91-93`, `:113-115`); `PRESENT_SQL` usa al agente como parte de la identidad de una fila ya importada (`import.rs:236-239`).
- `clawg memories list|search|stats --agent` filtran (`admin.rs:804-805`).

### Qué specs dicen la regla de hoy

| Spec | Requisitos | Qué dice |
|---|---|---|
| [`visibility`](../../../specs/visibility/spec.md) | R3, R4 y la definición «Llamada con scope» | El puente sólo lista y modifica lo propio; con scope, una fila ajena es 404. R1, R2, R5 y R6 (identidad, workspace) se quedan |
| [`sharing`](../../../specs/sharing/spec.md) | R2 (el detector `OwnOnly` y el escenario de la fila sin autor), R4 (la nota `SHARED`), R5 (lo que sigue siendo de cada agente y las descripciones de sólo lectura) y la definición de `OwnOnly` | R1, R3 y R6 a R9 se quedan |
| [`disputes`](../../../specs/disputes/spec.md) | «El detector anota candidatas» y su escenario, R6 (`agent_id`, 404 «de otro agente», línea de `memory_status`) y «Umbral y banda del detector» («del mismo agente», `OwnOnly`) | |
| [`ingest`](../../../specs/ingest/spec.md) | «Un save con clave reemplaza la fila viva de su clave» («Alcance y orden»: otro agente con la misma clave MUST NOT reemplazar), «Un duplicado devuelve la fila existente» | |
| [`restore`](../../../specs/restore/spec.md) | Definiciones (gemela viva, titular de la clave), R1 (404 de otro agente; titular del mismo «agente y proyecto»), R5 (descripción) | |
| [`update`](../../../specs/update/spec.md) | R3 (404 de otro agente) y R5 (herramienta) | |
| [`pieces`](../../../specs/pieces/spec.md) | R6, «El save entero» (`ids` en `null` de otro agente, 404) | |
| [`synthesis`](../../../specs/synthesis/spec.md) | Definiciones (alcance, pendiente), R3 (fuente de otro agente: 400), R7 (agente y escenarios), R8 (`unreviewed` del agente del pedido), «Conclusiones a revisar» | |
| [`mcp`](../../../specs/mcp/spec.md) | «Lo compartido en las herramientas», «La clave de tema en las herramientas» (el hit de otro agente no muestra `topic=`) | |
| [`retrieval`](../../../specs/retrieval/spec.md) | «Hits de otros agentes» (listado, síntesis, detector y ACTIVE CONFLICTS siguen siendo del agente), «Conflictos por referencia» | |
| [`doctor`](../../../specs/doctor/spec.md) | R6, filas `memory.topic` y `memory.content_hash`, y R7, fila `agents` | |
| [`portability`](../../../specs/portability/spec.md) | R5 (fundirse con el texto vivo y titular de la clave, «del mismo agente y proyecto») | |
| [`storage`](../../../specs/storage/spec.md) | Sección nueva para V20, con su plan de consulta y su vuelta atrás | |
| [`hooks`](../../../specs/hooks/spec.md) | R4 (el recordatorio de síntesis usa `unreviewed`) y R5: se revisan, no cambian | |

## Lo que pasa con una base existente

El cambio ensancha el alcance de las reglas de «agente y proyecto» a «proyecto». Una base de hoy puede tener lo que esas reglas, con el alcance viejo, dejaban convivir. Ninguna de estas situaciones hace perder, esconder ni destruir una fila: es lo que cada una hace con las reglas nuevas, y lo que se ofrece.

| Qué puede haber hoy | Qué hacen las reglas nuevas | Opciones |
|---|---|---|
| **(a)** Dos filas vivas con una misma `topic_key` en un proyecto, de dos agentes | Siguen vivas y alcanzables. El próximo save con esa clave en el proyecto reemplaza a las dos (`revise_in` recorre todo el alcance, `topic.rs:119-136`) y hereda el menor tier y los mayores accesos. Hasta entonces: borrar la vigente de una no revive nada mientras la otra sostiene la clave (`KEY_HELD_SQL`, `revival.rs:75-80`), y restaurar una borrada con la clave tomada da `TopicTaken` (`admin.rs:479-486`) | 1. Dejarlas, y que el doctor las marque (recomendado). 2. Un barrido al migrar que reemplace las más viejas: esconde memorias sin que nadie lo haya pedido y elige ganadora por la fecha y no por el tema; contradice la regla del proyecto. 3. Un comando explícito del usuario: queda para el reconciliador de 30 |
| **(b)** Filas vivas con un mismo `content_hash` en un proyecto, de agentes distintos | Siguen vivas. Un save del mismo texto refuerza la más vieja (`ORDER BY created_at ASC LIMIT 1`, `store_insert.rs:307`) y la otra queda como está | Igual que (a) |
| **(c)** Pares de `memory_conflict` entre agentes | No hay: el detector nunca los escribió. Las filas viejas de dos agentes no se emparejan solas, porque el detector corre al guardar y al restaurar | 1. No hacer nada (recomendado). 2. Un relleno que recorra el store: un KNN por fila y cientos de pares `pending` para que el agente juzgue, con las notas de 44 todavía por entrar; descartado |
| **(d)** `synthesis_review` por agente | No se tocan ni se borran. La revisión nueva se escribe con `agent_id = ''` por proyecto; al no existir esa fila, la primera lectura cuenta todo lo pendiente del proyecto como sin revisar (`synthesis.rs:268-286`), y si pasa de 20 `recall_context` y `session-start` avisan una vez, hasta el primer `synthesize` ([`specs/synthesis`](../../../specs/synthesis/spec.md) R8, [`specs/hooks`](../../../specs/hooks/spec.md) R4). Además la lista de pendientes ya incluye a todos los autores | 1. Ignorar las filas viejas (recomendado): un aviso de más y nada oculto. 2. Leer el `MAX(reviewed_at)` de los agentes del proyecto: callaría el aviso de filas que el agente que revisó nunca vio. 3. Migrar esas filas con SQL: cambia datos sin necesitarlo |
| **(e)** Filas `disputed` y pares `escalated` | Sin cambio de datos. ACTIVE CONFLICTS los muestra a todo agente que tenga un lado entre sus hits, y cualquiera puede registrar la respuesta del usuario con `user_answered` (la guarda de `dispute.rs:365-367` no cambia) | — |
| **(f)** Conclusiones (`memory_derivation`) de un agente | Cubren sus fuentes para todos; antes una conclusión ajena no cubría (`synthesis.rs:35`). Bajan las pendientes de los demás | — |

### ¿Hace falta una migración?

De índices, sí; de datos, no. `idx_memory_dedup` e `idx_memory_topic` empiezan por `agent_id`, y con las reglas por proyecto las búsquedas que los usan (filas 9, 10, 14, 15, 16 y 17 del inventario) no traen igualdad sobre esa columna. Opciones:

- **V20 que reemplaza los dos por versiones que empiezan por `COALESCE(project_id, '')`** (recomendado), como V18 reconstruyó el de dedup. No cambia ninguna fila. Un binario anterior la rechaza con `SchemaNewer`, la vuelta atrás es el snapshot `pre-migration` o el SQL que recrea los dos índices, y el modelo del test es `a_store_from_before_the_session_indexes_keeps_its_rows_and_gains_them` (`db.rs:1320`) con su plan de consulta (`specs/storage`, «Una base en V17 abre con los índices de las sentencias de sesión»). No se edita V6, V16 ni V18: sus checksums están fijados (`published_migration_checksums_are_pinned`, `migrations.rs:43`).
- **Sin migración**, con `agent_id IN (SELECT DISTINCT agent_id FROM memory)` en cada búsqueda para que el índice viejo sirva una vez por agente. Depende de que el planificador lo elija y paga un `DISTINCT` por guardado y por pieza; un save de 228 piezas lo repite 228 veces. No se recomienda.
- **V20 que suma los nuevos y deja los viejos**: la vuelta atrás es gratis, pero dos índices muertos encarecen cada escritura. No se recomienda.

## Lo que ya existe y se reutiliza

- **El modo sin scope de cada núcleo** (`owner: None`): `update_memory_core`, `delete_memory_core`, `restore_memory_core`, `judge_pair`, `read_save_core`, `existing_memory_ids` y `synthesis_core` ya valen sobre cualquier fila ([`specs/visibility`](../../../specs/visibility/spec.md) R4, «Sin scope»), y los tests de hoy los ejercen así. El cambio es dejar de pasarles el agente como dueño y pasarles, en su lugar, el alcance del lector.
- **`Reach`, `reach_predicate!` y `Reach::admits`** (`sharing.rs:42-63`, `search.rs:77-85`), con el test que mantiene iguales el predicado SQL y `admits` (`the_reach_predicate_agrees_with_admits`, `search.rs:5231`). Es el único criterio de «lo que un agente alcanza» y ya lo comparten la búsqueda y LAST SESSION.
- **`memory_project_predicate!`** (`search.rs:87-99`) y `Candidate.same_scope` (`contradiction.rs:84-87`): la noción de «por proyecto» del detector y de las claves ya existe.
- **`revise_in`, `revive_after_delete` y `plan_revival`** reciben un alcance como dato (`topic.rs:47-54`, `revival.rs:42-55`): cambiarlo es cambiar la consulta que lo lee.
- **V18** como molde de una migración de índices, con su prueba sembrada en la versión anterior, su plan de consulta y su vuelta atrás.
- **`display_author`, `is_shared` y la marca `by <autor>`** (`prompt_builder.rs:654-669`): el autor se sigue mostrando sin cambios.
- **`clawg sharing hide '*' '*'`**: deja a cada agente con lo suyo y lo que no tiene autor, y con la recomendación de abajo recupera también el aislamiento al modificar.

## Enfoques posibles

### A. Quitar el agente de cada regla y dejar que `sharing.json` acote lo que un agente alcanza (elegido)

Un solo criterio, el alcance del lector, vale para leer y para modificar; las reglas que comparan un save se aplican al proyecto dentro de ese alcance. El agente queda como autoría y como clave de las vistas por agente. Se hace en un solo cambio porque dejar la mitad produce un modelo con dos reglas: un agente podría borrar la fila de otro y su save no la reemplazaría ni la reconocería como repetida.

### B. Primero sólo el acceso, el alcance de las reglas después

Leer entero, cambiar, borrar, restaurar y juzgar sin frontera, que es lo que 44 necesita, y dejar dedup, detector, clave y síntesis por agente para un cambio posterior. Es más chico y destraba antes a 44. Pero deja el modelo a medias a propósito (las dos reglas conviven), toca dos veces las mismas specs, y el repetido y la doble versión vigente, que son el síntoma de que el cerebro es de varios, quedan sin resolver. Se descarta, aunque el orden de las tareas permite entregar el acceso primero dentro del mismo cambio.

### C. Mantener la frontera y sumar una forma de «adoptar» filas o de compartir la escritura

Por ejemplo una lista de escritores en `sharing.json` o un comando que reasigne autores. Conserva la protección de hoy, pero exige configurar cada instalación, suma un mecanismo de restricción nuevo y deja la regla «el autor decide» donde estaba. 44 la había previsto («adoptar las filas en un agente») y 76 la vuelve innecesaria. Se descarta.

### D. Un alias que una las identidades del usuario

`claude-code`, `mcp` y `migrated` pasarían a ser un agente. Resuelve el primer problema de visibility-policy y a 44 sin tocar reglas, pero no sirve entre Claude Code y otro cliente (objetivo 4), y agrega una tabla de alias que visibility-policy dejó fuera a propósito («normalizar o unir identidades de agente»). Se descarta.

## Decisiones tomadas

El 2026-10-22, antes de esta exploración:

- **`agent_id` dice quién escribió una fila y nunca quién puede leerla o cambiarla.** La frontera por agente nació de un problema que ya no existe (nada se compartía) y hoy sólo impide que un agente trabaje con lo que su búsqueda le muestra.
- **Cualquier agente lee entera, cambia, borra, restaura y juzga toda fila que alcanza, las de `agent_id` NULL incluidas.** Una fila sin agente ya se ve desde todos los agentes; que ninguno pueda tocarla hoy es una consecuencia de comparar con NULL, no una decisión.
- **Una fila sin proyecto la ven todos los agentes de todos los proyectos, como hoy.** Nada necesita restringirse y no se agrega un mecanismo de restricción.
- **Dedup, detector de contradicciones, clave de tema (con el revivir al borrar) y síntesis se aplican por proyecto, no por agente y proyecto.** Es lo que hace que el cerebro compartido tenga una versión vigente por tema y una fila por texto. «Por proyecto» significa lo que ya significa para cada una: igualdad de proyecto para el dedup y la clave, y el predicado de proyecto de la búsqueda (el proyecto, las filas sin proyecto y toda `user_preference`) para el detector.
- **Sigue por agente lo que describe su propia actividad:** el `workspace.md`, RECENT del contexto sin consulta, la sesión del host registrada (`POST /v1/sessions`) y el aviso del hook sobre el tiempo desde su último save.
- **El autor se sigue mostrando** (`by <autor>`): es información para quien lee, no una frontera.

## Decisiones abiertas

- [x] **1. ¿`sharing.json` acota también lo que un agente modifica?**
  - **a. Sí (recomendado).** Un agente cambia lo que alcanza: un autor oculto no se lee ni se cambia, y dedup, detector, clave y síntesis se aplican a *el proyecto dentro del alcance del que guarda*. Es la única forma de que `hide` siga significando algo cuando el usuario no quiere que un agente toque lo de otro, y de no imprimir texto oculto en `replaced`, `related`, ACTIVE CONFLICTS ni en el 400 de `derived_from`. Con `hide '*' '*'` vuelve el aislamiento de antes, ahora también al modificar. Costo: cada ruta que modifica necesita el alcance del lector (hoy lleva un `Option<&str>`); el worker tiene que leer `sharing.json` por save (hoy sólo el daemon lo lee, `api.rs:1310`); y «una versión viva por clave» pasa a valer por proyecto y alcance, así que dos agentes que se ocultan mutuamente pueden dejar dos titulares legítimos que el doctor no distingue de un defecto sin leer la política.
  - **b. No: `sharing.json` sólo acota lecturas.** Menos código. Pero un agente que no puede leer a otro lo reemplazaría, lo borraría o lo repetiría a ciegas, y las respuestas de los saves mostrarían su texto. Contradice el objetivo 4 («el usuario controla qué ve cada uno»).
  - **c. `sharing.json` define aparte quién escribe.** Un mecanismo de restricción nuevo, que lo decidido descarta.
- [x] **2. ¿Qué pasa con `?agent=` y los `agent_id` del cuerpo?**
  - **a. Se quedan aceptados y pasan a decir quién es el lector (recomendado)**, para aplicar su alcance (decisión 1.a). Sin ellos la operación es del usuario y no tiene frontera, como hoy. Un puente anterior sigue funcionando (premisa del `deny_unknown_fields`). Costo: `OwnerQuery` y los «owner» de los núcleos mienten, y se renombran sin cambiar el nombre del parámetro.
  - **b. Se quitan.** Más simple, pero sin lector no hay alcance (1.a) y un puente anterior recibe un 400.
  - **c. Se quedan como filtro de autor opcional.** Sirve al visor, que ya lo tiene en `list` y `stats`; en las rutas por id no hace falta.
- [x] **3. ¿Cómo se cuentan los pares por proyecto en `memory_status`, ACTIVE CONFLICTS y el recordatorio?**
  - **a. (recomendado)** `memory_status` cuenta los pares cuya fila nueva está en el proyecto del puente y cuyos dos lados alcanza el lector (sin proyecto, todo el store, como `synthesize`); `GET /v1/disputes` suma `project_id`. ACTIVE CONFLICTS quita la condición de agente, conserva la de estar entre las filas mostradas y exige que el lector alcance los dos lados. `unreviewed` y `conclusions_to_review` se cuentan por proyecto.
  - **b. Todos los pares que alcanza el lector, sin proyecto.** Más sencillo, pero arrastra pares de proyectos donde no está trabajando.
  - **c. Seguir con «los pares que abrieron los saves del agente».** Conserva a quién se le pidió juzgar y no agrega ruido, pero un par que abrió un agente que ya no trabaja no lo ve nadie, que es lo que el modelo nuevo quiere evitar.
- [x] **4. ¿Qué lista `list_memories`?**
  - **a. Todo lo que alcanza, lo más nuevo primero, con ` by <autor>` en lo ajeno y un argumento opcional `agent` (recomendado).** El inventario pasa de «mis filas» a «las del cerebro», que es lo que `read_memory` y `delete_memory` ya aceptan, y el filtro devuelve lo de antes en una llamada. Cambia el contrato y el test que fija los cinco argumentos (`server.rs:2106-2140`), cuyo motivo («leer entre agentes») ya no vale; el buscador de texto del visor sigue sin exponerse. La marca ` by <autor>` no debe confundirse con la ` by=#N` de una fila reemplazada (`tools.rs:329-338`).
  - **b. Sólo las propias por defecto y un `all_agents`.** No cambia lo que el modelo ve hoy, pero pide un argumento para ver lo que las demás herramientas ya muestran.
  - **c. Todo, sin argumento nuevo.** Con 3.713 notas de `migrated` el modelo no puede acotar el listado.
- [x] **5. ¿Cómo se nombra la regla del doctor?**
  - **a. Mismos `check_id` y `reason_code`, nuevo alcance (recomendado).** `memory.topic` y `memory.content_hash` agrupan por proyecto; el mensaje y la evidencia cambian (`agents` con los autores implicados en lugar de `agent_id`). Los ids nombran la regla, no su alcance, y quien filtre por ellos no se rompe. En una base existente avisan de (a) y (b), sin `safe_next_step`, como hoy.
  - **b. Ids nuevos** (`memory.topic_project`): rompe a quien filtre por id y deja dos reglas en la spec.
  - **c. Las dos reglas a la vez**: redundante, la de agente y proyecto queda contenida en la de proyecto.
  - El hallazgo `fallback_agent` se queda con otro texto y el mismo `warn`, porque las filas bajo `mcp` siguen fuera del workspace y RECENT del agente real.
- [x] **6. ¿La guarda de `user_answered` llega a borrar y a cambiar la importancia de una fila protegida ajena?** (la encontró la exploración)
  - **a. No en este cambio (recomendado).** Borrar es un soft delete que `restore_memory` y `undelete` deshacen, y la guarda de 63 existe para que un agente no reemplace la palabra del usuario con un veredicto; borrar una fila propia tampoco la tenía.
  - **b. Sí, sólo para una fila protegida de otro autor.** Cierra el caso de un agente que borra la preferencia que otro guardó por pedido del usuario. Cuesta un argumento más en `delete_memory` y `update_memory` y otro cambio de contrato.
  - **c. Sí, para toda fila protegida.** Cambia el comportamiento de borrar lo propio.

## Relación con otros cambios

| Cambio | Relación |
|---|---|
| **30 project-clone-identity** | «Unir dos scopes sin reconciliar» ([exploración](../../project-clone-identity/exploration.md), líneas 50-57) describe el mismo defecto: dos titulares de una clave y dos filas con un mismo texto. Ensanchar el alcance de «agente y proyecto» a «proyecto» es unir scopes (situaciones (a) y (b) de arriba). El reconciliador que 30 propone (Q1: `revise_in` por clave repetida y refuerzo de los duplicados exactos, líneas 162-165) se escribe una vez, por proyecto; sus ejemplos de «mismo agente» pasan a «mismo proyecto». Si 76 entra primero, 30 lo hereda. |
| **37 knn-prefilter** | Su premisa del detector y su caso concreto ([exploración](../../knn-prefilter/exploration.md), líneas 19 y 48) suponen `Reach::OwnOnly`: las filas de otros agentes ocupan la ventana de 5 y de 20 y dejan sin ver la fila propia. Con 76 esas filas son candidatas, no ruido: el crowding del detector pasa a venir de otros proyectos y de las revisiones `superseded`. Su aplicación al detector «sin cambiar su alcance» (línea 162) y su test 4, «no marca nunca una fila ajena» (línea 205), dejan de valer, y la metadata `agent_id` de su enfoque A (línea 95) sería de proyecto y alcance. 76 no depende de 37; 37 se relee después de 76. |
| **44 history-import** | Depende de este cambio ([propuesta](../../history-import/proposal.md), líneas 25 y 47). Lo que 76 le da: leer enteras sus notas largas, corregirlas, borrarlas y juzgarlas desde el agente del usuario. Lo que le cambia: el import de 43 que usa (`place`) mira el proyecto y no el agente para el texto vivo y el titular de la clave (fila 17); el detector del que guarda empieza a emparejar saves con las notas de `migrated` del proyecto (más candidatas `pending` por guardado); el dedup absorbe en una fila de `migrated` un texto idéntico; y la lista de pendientes de `synthesize` incluye sus notas (ver «Riesgos»). Las notas sin proyecto sólo entran en el alcance de `synthesize` sin proyecto (`synthesis.rs:9-12`) hasta que 77 las acomode. |
| **77 project-reassign** | Mover una fila a otro proyecto bajo las reglas por proyecto puede dejar, en el destino, dos titulares de una clave o dos filas con un mismo texto. 77 tiene que reconciliar o rechazar eso; el alcance que respeta lo fija 76. |
| **68 topic-key-warning** | En verificación. Su aviso de que un save sin clave quizá revisa una fila se midió con candidatas de un solo agente: 154 de 220 saves, el 49,4 % revisiones y 199 de 204 etiquetas de un agente (README, fila 48). Con 76 la fila que nombra puede ser de otro agente; la precisión de ese caso no está medida (`revision-eval warned` lo mide). |
| **63 protected-verdicts** | Su guarda cubre reemplazar y sacar el tipo; no cubre borrar (decisión abierta 6). |

## Alcance candidato

1. Un solo criterio de alcance del lector para leer y modificar: `read_save_core`, `update_memory_core`, `delete_memory_core`, `restore_memory_core`, `judge_pair` y `resolve_dispute_core`, y la comprobación de `derived_from`. Los parámetros `owner` pasan a ser el alcance del lector (decisiones 1 y 2); la ruta `?agent=` y los `agent_id` del cuerpo se siguen aceptando.
2. Las reglas por proyecto, dentro del alcance del que guarda: dedup (y su gemela al restaurar y el import), detector (con `OwnOnly` fuera, también al restaurar), clave de tema (con el revivir al borrar, el titular al restaurar y el import) y síntesis (pendientes, conclusiones, revisión y recordatorio por proyecto).
3. V20: los índices de dedup y de clave por proyecto, con su prueba sembrada en V19, su plan de consulta, su checksum y su vuelta atrás.
4. Las vistas: ACTIVE CONFLICTS, los pares de `memory_status`, `list_memories` con el autor y el argumento `agent` (decisiones 3 y 4), la clave visible en cualquier hit del proyecto, y la línea `shared:` revisada.
5. Lo que ve el modelo: las cuatro descripciones, `SHARED_NOTE`, `protocol.md` (con la advertencia de las claves compartidas) y las instrucciones, con `agent_contract.json` bendecido.
6. El doctor: `memory.topic`, `memory.content_hash` y el texto de `fallback_agent` (decisión 5).
7. Los trece specs de arriba y `.claude/CLAUDE.md`; los tests que fijan la regla vieja se reescriben en el commit que cambia el código.

### No-alcance

- **Mover filas entre proyectos:** es project-reassign (77), explorado aparte. 76 sólo fija el alcance por proyecto que 77 tiene que respetar.
- **Adoptar filas en un agente o renombrar autores:** deja de hacer falta, y un alias de identidades se descartó (enfoque D).
- **Un mecanismo de restricción nuevo:** grupos, permisos de escritura o una lista de escritores en `sharing.json`.
- **Reconciliar lo que una base existente tiene repetido entre agentes** (situaciones (a) y (b)): sólo el doctor lo dice. El reconciliador, si se quiere, es el de 30.
- **Rellenar pares entre filas viejas de agentes distintos** (situación (c)).
- **Una guarda de `user_answered` para borrar** (decisión 6 en su opción a).
- **El prefiltro de vec0 (37) y el ranking:** la búsqueda no se toca, así que `rag-eval` no aplica.
- **Que el `workspace.md` de un agente incluya las `user_preference` de importancia alta que guardó otro:** es consecuencia de dejar el workspace por agente, que está decidido; se anota para quien lo mida después.

## Cómo se mide

Tests RED primero, con dos agentes A y B sobre un store de prueba y sobre el daemon en proceso (`crates/clawg-cli/tests/api.rs`); la lista final va en la propuesta. Fallan hoy:

1. B lee entero un save largo de A y una fila sin autor (`read_memory` y `GET /v1/memories/{kind}/{id}/save`), y un pedazo ajeno ya no figura como faltante.
2. B cambia, borra, restaura y juzga una fila (o un par) de A y una fila sin autor; el cambio queda hecho y A lo ve.
3. Con `hide B→A`, todo lo anterior responde «no existe» y no cambia nada, y A sigue pudiendo con lo de B. Un `sharing.json` ilegible deja a B con lo suyo y lo que no tiene autor.
4. Dedup: B guarda el texto de A en el mismo proyecto y obtiene `reinforced` con el id de A; en otro proyecto, o con `hide`, se inserta.
5. Clave: el save con clave de B reemplaza al titular de A en el proyecto y lo lista en `replaced`; en otro proyecto no; borrar la vigente de B revive la de A (causa `revision`); restaurar la de A con la de B vigente da `TopicTaken`.
6. Detector: B guarda en la banda de una fila de A y obtiene el par y `related`; con `hide` no hay par; una fila sin autor se empareja; `revision_candidate` puede nombrar la fila de A.
7. Síntesis: `synthesize` de B lista las pendientes de A; una conclusión de B con `derived_from` de filas de A las cubre; la revisión es del proyecto y el aviso se apaga para los dos.
8. ACTIVE CONFLICTS: un par entre A y B sale en el prompt de los dos; con `hide` el texto del lado oculto no sale.
9. `memory_status` cuenta por proyecto; `list_memories` lista todo con el autor y `agent` filtra.
10. Lo que no cambia: el workspace, RECENT, la sesión registrada y el aviso del hook siguen siendo de cada agente (los tests de hoy pasan sin tocarlos).
11. Un puente anterior que manda `?agent=` no recibe 400.
12. Las descripciones, `SHARED_NOTE`, `protocol.md` y el contrato no dicen «sólo de lectura»; las instrucciones siguen en 2.048 caracteres o menos.
13. Doctor: `memory.topic` y `memory.content_hash` agrupan por proyecto y nombran a los autores.

Pasan hoy y dejan de valer, así que se reescriben en el commit que cambia el código: `an_agent_cannot_delete_another_agents_memory`, `an_agent_cannot_update_another_agents_memory`, `an_agent_cannot_restore_another_agents_memory`, `an_agent_cannot_read_another_agents_save`, `an_agent_cannot_resolve_another_agents_conflict`, `an_agent_lists_only_its_own_memories`, `a_shared_memory_is_read_only` y `tool_descriptions_say_shared_memories_are_read_only` (`server.rs`); `delete_with_another_agent_is_404_and_keeps_the_row` y `an_agent_neither_reviews_nor_cites_another_agents_memories` (`tests/api.rs`); `reinforce_if_duplicate_other_agents_row_is_not_reinforced` (`store_insert.rs`); `a_key_is_scoped_to_its_agent_and_project` (`worker.rs`); `contradiction_probe_ignores_shared_authors` y `a_save_is_never_paired_with_a_row_no_agent_saved` (`contradiction.rs`); `another_agents_conclusion_does_not_cover_a_memory`, `synthesis_stays_with_its_agent` y `existing_ids_leave_out_another_agents_rows` (`synthesis.rs`); `a_pair_of_another_agent_is_not_found` (`dispute.rs`); `restoring_needs_a_deleted_row_of_the_owner` (`admin.rs`); la mitad de ACTIVE CONFLICTS de `the_workspace_and_conflicts_stay_own` (`prompt_builder.rs`); `another_agents_hit_does_not_show_its_topic_key` (`tools.rs`). Cada uno pasa a afirmar lo contrario y, con `hide`, lo de antes.

**Datos de la versión anterior** (la regla del proyecto para todo cambio de esquema): una prueba como `a_store_from_before_the_session_indexes_keeps_its_rows_and_gains_them` siembra con SQL directo una base en V19 con las situaciones (a) a (f) —dos agentes con la misma clave en un proyecto, dos filas con un mismo hash, `synthesis_review` por agente, una fila `disputed` con su par escalado, una conclusión y una fila sin autor—, la abre con `MemoryStore::open` (así pasa por el snapshot `pre-migration`) y prueba que cada fila queda igual y alcanzable desde el otro agente (búsqueda, listado y lectura entera), que los índices nuevos existen y que el plan de dedup y el de clave los usan sin `TEMP B-TREE`, y que el primer save con la clave reemplaza a los dos titulares. Suma el checksum de V20 y, como en V17, un escenario de `clawg update` sobre una base en V19 ([`specs/self-update`](../../../specs/self-update/spec.md), S10 como modelo).

`rag-eval` no aplica: el ranking no se toca. `conflict-eval` (un solo agente) y `revision-eval chains` deben dar los mismos números. A mano y con autorización, sobre el perfil `dev`: dos puentes con `--agent` distintos; uno guarda una nota larga, el otro la lee entera, la corrige, la borra y la restaura; `clawg sharing hide` corta cada una de esas operaciones; y `clawg doctor` sobre una base de la versión anterior con las situaciones (a) y (b).

## Riesgos

**Lo que protegía la frontera y lo que queda.** La frontera hacía que un agente comprometido (una memoria envenenada que lo manda a borrar) sólo pudiera tocar lo suyo. Con el cambio alcanza todo lo que lee. Queda: el soft delete, que `restore_memory` y `undelete` deshacen (con sus rechazos: gemela viva, clave tomada, secreto); la guarda de 63 sobre reemplazar y degradar una `user_preference` o una `architectural_decision` (`dispute.rs:365-367`, `admin.rs:266-269`); `sharing hide` para el agente que no es de fiar; los snapshots `pre-migration`, `pre-reembed` y `pre-import` y `clawg backup`; y que cada herramienta toca una fila por llamada, que queda en el registro de la sesión. No queda: un snapshot antes de cada borrado, ni nada que impida a un agente borrar la preferencia que otro guardó por pedido del usuario (decisión 6). `clawg purge` sigue siendo del usuario: pide el daemon parado.

| Riesgo | Prob. | Mitigación |
|---|---|---|
| Un agente comprometido o confundido borra o cambia lo de todos | Media | Lo de arriba; `sharing hide '*' '*'` vuelve al aislamiento; decisión abierta 6 |
| Dos agentes usan una clave genérica (`notes`, `conventions`) para temas distintos, y el segundo reemplaza al primero sin que ninguno lo sepa | Media | La respuesta del save lista lo reemplazado con su primera línea (`replaced`); borrar la versión nueva revive la anterior (`revival.rs`); `protocol.md` dice que la clave nombra un tema del proyecto. Podría sumarse el autor a esa línea |
| Texto de una fila oculta impreso en `replaced`, `related`, ACTIVE CONFLICTS o el 400 de `derived_from` | Alta si se elige 1.b | Decisión 1.a: todo lo que se imprime pasa por el alcance del lector, y una prueba por cada salida |
| Las búsquedas de dedup y de clave recorren todo el índice por falta de uno que empiece por proyecto | Alta sin V20 | V20 y el plan de consulta fijado; una pieza de un save largo repite la búsqueda |
| El dedup absorbe un texto de B en una fila de A: el RECENT y el aviso del hook de B no lo cuentan como suyo, y los accesos de A suben | Baja | Es idéntico a lo que hace hoy un agente con su propio texto; los accesos son un uso real. Se documenta |
| El detector del que guarda empieza a emparejarse con las notas de `migrated` (44) y cada save trae más candidatas `pending` | Media | La banda y el piso no cambian; `conflict-eval` sobre un flujo multiagente lo mide antes de 44; el agente juzga con `resolve_conflict` |
| `synthesize` pasa a listar las miles de notas de `migrated` como pendientes y el aviso de 20 salta en cada proyecto | Media | El aviso se apaga con el primer `synthesize`; `pending_total` es un conteo y la lista se acota con `limit`. Es lo que el objetivo 5 pide para esas notas, y se mide en 44 |
| Un puente y un daemon de versiones distintas: un daemon anterior con un puente nuevo sigue aplicando la frontera vieja y contesta «no existe» | Baja | `clawg update` los reemplaza juntos; `version_warning` avisa en el visor y no en el puente ([`specs/mcp`](../../../specs/mcp/spec.md), «Reconectar con el daemon»). El diseño decide si el puente aconseja `clawg stop` ante un 404 sobre una fila ajena, como hace `route_failure_text` (`server.rs:923-933`) |
| Un agente sube la importancia o cambia el tipo de la fila de otro y la lleva a su workspace (importancia 8 o más en tier 1) | Baja | El contenido lo escribió su autor; la fila sigue marcada por la guarda de 63 si es protegida. Queda anotado para la decisión 6 |
| La precisión del aviso de 68 no cubre candidatas de otro agente | Media | `revision-eval warned` después de 76; el aviso se puede quitar sin tocar datos |
| Muchos specs y tests fijan la regla vieja y se pueden dejar a medias | Media | La lista de «Cómo se mide» y los trece specs de arriba; cada commit atómico lleva su código, su test y su spec |
