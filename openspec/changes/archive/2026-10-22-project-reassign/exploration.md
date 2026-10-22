# project-reassign — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 2 | M, partible en dos: el motor con la puerta de una fila, y la puerta masiva con su vuelta atrás | 76 agent-authorship, blanda: cambia quién puede mover y el ámbito de los choques, no la forma del cambio. Lo pide 44 (history-import), que debe guardar el nombre del proyecto de origen ([requisito sobre 44](#lo-que-77-necesita-de-44)) | exploración |

**Tesis:** una fila de `memory` queda para siempre en el proyecto con que se guardó. Ni `PATCH /v1/memories/{kind}/{id}` ni `update_memory` cambian `project_id`, el visor ni siquiera puede listar las filas sin proyecto, y lo único que hoy mueve filas es el re-key de `POST /v1/projects`, que traslada todas las de un id viejo sin mirar choques. El cambio agrega *mover*: poner una fila sin proyecto en uno, sacar una del proyecto equivocado, o dejarla sin proyecto. La operación cambia `project_id` (y `scope` cuando el CHECK de la tabla lo exige) y nada más: no re-embebe, no toca el texto, no corre el detector. Lo que sí cuida es todo lo que `project_id` decide: el save se mueve entero, la cadena de revisiones de una clave se mueve con su fila vigente, y ninguna fila llega a un proyecto donde dejaría dos vivas con una clave o con un texto. Se hace con un solo motor y dos puertas: `PATCH`/`update_memory` para un save, y un comando masivo con `--dry-run`, snapshot y registro por fila para lo que trae 44, cuyas notas entran en un 43 % (1.594 de 3.713) sin proyecto y "se acomodan después". Esto es ese después. Una fila sin proyecto se sigue viendo desde todos los proyectos: no se restringe nada, sólo se permite reubicar.

## Premisas que se verificaron

| Premisa | Resultado |
|---|---|
| `PATCH` y `update_memory` sólo cambian `importance`, `memory_type` y `user_answered` | **Cierta.** `UpdateRequest` tiene `deny_unknown_fields` (`crates/clawg-cli/src/daemon/api.rs:467-481`), así que un campo de proyecto hoy se rechaza. `update_memory_core` escribe `importance`, `memory_type`, `tier`, `last_modified_at` y `update_count` (`crates/clawg-core/src/memory/admin.rs:220-222`). Los argumentos del puente son los mismos (`UpdateMemoryArgs`, `crates/clawg-cli/src/mcp/server.rs:309-328`). |
| El resto del sistema tampoco cambia el proyecto de una fila | **Cierta, con una excepción.** El re-key de `register_project` hace `UPDATE {tabla} SET project_id = ?2 WHERE project_id = ?1` para **todas** las filas de cada id viejo, borradas incluidas (`crates/clawg-core/src/memory/project.rs:116-128`). El import sólo asigna proyecto al insertar (`prepare`, `crates/clawg-core/src/memory/portability/import.rs:149-162`); `--map-root` nunca toca una fila ya guardada. |
| Una fila sin proyecto se ve desde todos los proyectos | **Cierta.** `memory_project_predicate` admite `project_id = ?` o `project_id IS NULL` o `memory_type = 'user_preference'` (`crates/clawg-core/src/memory/search.rs:92-99`), y se usa en el KNN (`search.rs:257`), el conteo de filas elegibles (`:325`), el canal léxico (`:754`), las coincidencias de identificadores (`:890`) y la puntuación de los candidatos léxicos (`:1050`), y en RECENT (`crates/clawg-core/src/memory/prompt_builder.rs:905-919`). Sin proyecto (`all_projects`) lee todas. |
| Una fila sin proyecto se ve en todos los contextos | **Falsa para dos lecturas.** LAST SESSION pide `project_id IS ?1` exacto (`prompt_builder.rs:809-817`; [`specs/retrieval`](../../../specs/retrieval/spec.md), «La última sesión»): una `session_summary` sin proyecto no es el último traspaso de ningún proyecto. `synthesize` por proyecto filtra `project_id = ?` (`crates/clawg-core/src/memory/synthesis.rs:146-149`) y el módulo lo dice: «rows without a project belong only to the whole store» (`synthesis.rs:9-12`). |
| El proyecto de una fila decide el ámbito de `topic_key`, del dedup y del detector | **Cierta.** Clave: `agent_id = ?1 AND COALESCE(project_id, '') = COALESCE(?2, '') AND topic_key = ?3` (`crates/clawg-core/src/memory/topic.rs:83-86`, índice `idx_memory_topic`, `crates/clawg-core/migrations/V16__topic_key.sql:13-15`). Dedup: el mismo patrón con `content_hash` (`REINFORCE_LOOKUP_SQL`, `crates/clawg-core/src/memory/store_insert.rs:300-307`, índice en `V18__session_indexes.sql:22-24`). Detector: busca con el proyecto del save, las filas sin proyecto y las preferencias, sólo entre las filas del que guarda (`crates/clawg-core/src/memory/contradiction.rs:172-184`); `Candidate.same_scope` es sólo la marca de que el candidato tiene el mismo proyecto (`contradiction.rs:87`, `:225`), y alimenta el aviso de clave (`unkeyed_revision`, `topic.rs:179-184`). |
| Las piezas de un save deben moverse juntas | **Cierta, y la tabla no lo impone.** `memory_piece` no tiene proyecto (`V11__memory_piece.sql:11-17`): cada pieza es una fila con su propio `project_id`, y leer el save une las posiciones por `save_id` sin mirar proyectos (`crates/clawg-core/src/memory/pieces.rs:136-148`). Mover una sola dejaría un save partido entre dos proyectos. Además una fila puede estar en **varios** saves (una pieza que el dedup absorbió; `pieces.rs:213-215`, `:243-264`). |
| Una revisión de clave se deshace al borrar la vigente | **Cierta, y exige el mismo ámbito.** `REPLACED_SQL` sólo cuenta como revisión una fila con causa `revision`, del mismo agente y proyecto y con clave (`crates/clawg-core/src/memory/revival.rs:66-73`); `KEY_HELD_SQL` mira el mismo ámbito (`revival.rs:77-80`). Si la vigente se mueve sola, borrarla después ya no revive la versión anterior: queda en otro proyecto. |
| Los pares y las derivaciones dependen del proyecto | **Falsa.** `memory_conflict` y `memory_derivation` guardan ids, sin proyecto (`V15__memory_conflict.sql:11-21`, `V10__synthesis.sql:8-13`). Los pares abiertos se cuentan por agente (`OPEN_PAIR_FROM`, `crates/clawg-core/src/memory/dispute.rs:440-446`), ACTIVE CONFLICTS se arma por los ids que el prompt muestra (`prompt_builder.rs:1055-1081`) y `judge_pair` no compara proyectos (`dispute.rs:325`). Un par entre dos proyectos se puede juzgar igual. |
| El workspace sigue al proyecto | **Cierta a medias.** El AUTO se regenera en cada contexto (`regenerate_auto_section`, único llamador `crates/clawg-cli/src/daemon/api.rs:1337`) con las decisiones de importancia 8 o más del proyecto **o sin proyecto** (`crates/clawg-core/src/memory/workspace.rs:281-291`), las preferencias por agente sin mirar proyecto (`workspace.rs:255-264`) y las habilidades (`:335-346`), así que no hay nada que reparar. La sección USER es un archivo por agente y proyecto (`workspace.rs:44-49`) y no sigue a las filas. |
| Mover una fila pide tocar vectores o el índice de texto | **Falsa.** `memory_vectors` es `vec0(embedding float[384])` sin columnas de proyecto (`V2__vectors.sql:6-7`) y `memory_fts` es de contenido externo: lee `project_id` de `memory` al consultar (`V8__memory_fts.sql:20-22`); sus tres triggers son de insert, de borrado suave y de borrado (`V8:63-88`), ninguno de `project_id`. `project_rekey` ya lo prueba (`a_moved_row_is_found_lexically_under_its_new_project`, `project.rs:290`). El índice de docs es derivado del disco y no toca `memory` ([`specs/project-docs`](../../../specs/project-docs/spec.md)). |
| El registro de re-key (`project_rekey`) sirve para registrar un movimiento | **Falsa para el caso principal.** `legacy_id` y `project_id` son `NOT NULL` (`V9__project.sql:17-24`): no puede decir «venía de sin proyecto» ni «fue a sin proyecto», que es el motivo del cambio. Además sólo anota cambios de `project_id`. |
| La columna `scope` se relaciona con `project_id` | **Cierta.** El CHECK exige `agent` con agente, `project` con proyecto y sin agente, y `global` sin agente ni proyecto (`V1__init.sql:30-34`); `check_memory` repite la regla para el import (`crates/clawg-core/src/memory/portability/format.rs:242-250`). Un save nuevo siempre es `scope = 'agent'` (`store_insert.rs:221`) y el import conserva el de la línea (`import.rs:320`), así que `project` y `global` sólo existen en filas sin agente, no en lo que escribe el producto. Mover una fila sin agente a o desde «sin proyecto» obliga a cambiar `scope`. |
| Un id cualquiera sirve de destino sin consecuencias | **Falsa.** `project` guarda `(id, root_path)` (`V9__project.sql:6-10`) y el id sale de la raíz (`derive_project_id`, `crates/clawg-core/src/memory/identity.rs:11-15`); una raíz cuyo id es de otra responde 409 `project_root_collision` (`project.rs:97-99`, `api.rs:1731-1734`, [`specs/project`](../../../specs/project/spec.md) R4). Hoy ni el save (pasa el `project_id` tal cual, `api.rs:1177`) ni `clawg doctor` (sólo compara cada id de `project` con su raíz, `crates/clawg-cli/src/doctor/checks.rs:372-392`) comprueban que el `project_id` de una fila esté en `project`. Una fila movida a un id con un error de tipeo quedaría invisible salvo con `all_projects`, sin ningún aviso. |
| El visor puede listar lo que hay que acomodar | **Falsa.** El filtro de proyecto es `project_id = ?` (`admin.rs:873-876`; `ListQuery`, `api.rs:418-443`); no existe «sin proyecto». Sólo `stats` cuenta el grupo sin proyecto (`Count { key: None }`, `admin.rs:1226-1232`, `:1276`). |
| Un agente solo cambia lo suyo | **Cierta hoy.** Con `?agent=`, la fila de otro es un 404 (`UPDATE_READ_MEMORY`, `admin.rs:213-214`; [`specs/visibility`](../../../specs/visibility/spec.md) R3 y R4); sin el parámetro la operación vale sobre cualquier fila (`OwnerQuery`, `api.rs:456-465`), que es la puerta del comando del usuario (`crates/clawg-cli/src/undelete.rs:34`). Las filas del agente `migrated` de 44 sólo las puede mover, entonces, el comando del usuario. |
| 44 guarda de dónde viene cada nota | **Cierta a medias.** Su propuesta pone en `created_by_ref` «la fuente y el id de la nota» y `source = 'import'` ([propuesta](../../history-import/proposal.md), «Decisiones», «Origen»), y el nombre de proyecto de la fuente sólo sirve para el mapa de la corrida: un nombre sin mapa entra sin proyecto y no deja rastro. `created_by_ref` se lee sólo para mostrarlo (`crates/clawg-cli/src/memories.rs:212-216`) y ningún filtro lo alcanza. |
| Tras mover, un segundo import no duplica | **Falsa para un export de clawg.** Una fila está presente si coinciden agente, proyecto, hash y fecha (`PRESENT_SQL`, `import.rs:236-239`); la movida ya no tiene el proyecto del archivo, y el chequeo de gemelo vivo (`TWIN_SQL`, `import.rs:242-245`) mira el proyecto del archivo, donde ya no está. Se inserta otra copia. Sólo el reconocimiento por origen que propone 44 lo evita, y sólo para lo que viene de otra herramienta. |

## Problema (verificado contra el código)

### Hoy no hay forma de acomodar una fila

- Una fila guardada sin proyecto, o en el proyecto equivocado, sólo se acomoda con SQL a mano y el daemon parado, saltándose el save entero, la cadena de claves, los choques y cualquier registro.
- 44 va a traer unas notas que entran sin proyecto por construcción: 1.594 de 3.713 (43 %) no lo traen ni en la nota ni en su sesión, y los nombres sin mapa (hasta 21 distintos) entran igual. Su propuesta los deja «para otro cambio».
- Sin proyecto no es neutro: la fila ensucia el KNN de todos los proyectos, es candidata del detector de todos, nunca es LAST SESSION y nunca entra en la síntesis de un proyecto. Es lo que se quiere corregir, no lo que se quiere prohibir: la decisión de que siga viéndose desde todos queda como está.

### Lo que decide `project_id`, y qué pasa al mover

| Qué | Dónde | Al mover una fila de A a B |
|---|---|---|
| Búsqueda y RECENT | `search.rs:92-99`; `prompt_builder.rs:905-919` | De sin proyecto a B: deja de verse desde cualquier otro proyecto y se ve desde B (y con `all_projects`). De A a sin proyecto: se ve desde todos. Una `user_preference` se ve desde todos antes y después. Sin trabajo de vectores ni de texto. |
| LAST SESSION | `prompt_builder.rs:809-817`, índice `V18__session_indexes.sql:9-11` | Una `session_summary` que llega a B es su último traspaso si es la más nueva viva de B, sin umbral de antigüedad ([`specs/retrieval`](../../../specs/retrieval/spec.md), «La última sesión»): un resumen viejo de una nota importada puede ser el único que B tiene. Deja de serlo en A. |
| Workspace AUTO | `workspace.rs:281-291`, `:44-49` | Las decisiones importantes salen del AUTO de A y entran en el de B en el próximo contexto. La sección USER no se mueve. |
| `topic_key` | `topic.rs:83-86`, `:111-150`; `revival.rs:66-80`; `import.rs:247-250` | La clave pasa a revisar y a ser revisada en el ámbito de B. Si B ya tiene una fila viva con la clave, quedarían dos vigentes (`clawg doctor` lo marca, `crates/clawg-core/src/memory/inspect.rs:233-260`). |
| Dedup | `store_insert.rs:300-307`; `import.rs:242-245` | Un re-save del mismo texto en B refuerza la fila movida. Si B ya tiene una viva con ese hash, quedarían dos (`inspect.rs:291-314`). |
| Detector | `contradiction.rs:172-184` | La fila entra al pool de los saves de B (y sale del de A, o entra en el de todos si pasa a sin proyecto). Mover no escribe pares: no corre. |
| Piezas | `V11__memory_piece.sql:11-17`; `pieces.rs:136-148`, `:213-215` | Todas las filas del save, y de todo save que comparta una fila con él, se mueven juntas. |
| Cadena de revisiones | `revival.rs:42-73`; `V17__revival.sql:9-18` | La vigente y las que reemplazó (causa `revision`, misma clave) se mueven juntas, o el borrado de la vigente deja de revivir la anterior. |
| Pares y derivaciones | `V15`, `V10`; `dispute.rs:440-446`; `prompt_builder.rs:1055-1081` | Siguen valiendo por id. Pueden quedar entre dos proyectos; se juzgan igual. `disputed` se conserva. |
| Síntesis | `synthesis.rs:28-39`, `:135-150`; `V10:22-27` | La fila pasa a `pending` de B. `unreviewed_count` cuenta por `created_at` posterior a la última revisión (`synthesis.rs:268-286`): una fila vieja movida figura en `pending` pero no como novedad. |
| `last_modified_at` de una fuente reemplazada | `synthesis.rs:288-292`, `:311-315` | Es el momento en que se perdió la fuente. Mover una fila `superseded` no puede escribirlo: haría reaparecer «conclusiones por revisar». |
| `scope` | `V1__init.sql:30-34`; `format.rs:242-250` | Una fila sin agente cambia `scope` con el proyecto; una con agente (todas las del producto) no. |
| Hook `user-prompt-submit` | `crates/clawg-cli/src/hook.rs:172-180` | Cuenta como último save del agente en B la fila más nueva por `created_at`: una fila reciente movida cuenta. |
| `export --project` | [`specs/portability`](../../../specs/portability/spec.md) R2 | Exporta por el proyecto actual de la fila. |

No cambian: vectores, índice de texto, tier, importancia, accesos, `source_session_id`, `created_by_*`, el agente, el índice de docs ni `sharing.json` (que habla de agentes, no de proyectos).

### Choques al llegar a un proyecto

| Caso | Qué hace hoy el código parecido | Qué se propone |
|---|---|---|
| Una viva del destino tiene la misma `topic_key` | `restore`: 409 `TopicTaken`, la borrada sigue borrada (`admin.rs:479-486`, `:678-682`, `:416`). Import: la entrante se inserta `superseded` por la del destino, causa `revision` (`import.rs:300-316`). Re-key: no mira (`project.rs:116-128`; [30](../../project-clone-identity/exploration.md)). | Rechazar el save sin escribir nada, nombrando la fila que sostiene la clave. |
| Una viva del destino tiene el mismo hash enmascarado | `restore`: 409 `LiveTwin` (`admin.rs:468-473`, `:675-677`, `:414`). Import: se funde con la del destino (`import.rs:295-299`). Re-key: no mira. | Rechazar el save nombrando el gemelo. Para resolver: `delete` de uno (se trae con `undelete`) y repetir. |
| La fila está en un par | Los pares son ids. | Se queda como está; el reporte lista los pares que quedan entre dos proyectos. `disputed` se conserva. Sin detector. |
| La fila está borrada | `restore` valida en el proyecto actual de la fila. | Se mueve con su save, sin choque (no está viva), y un `undelete` posterior valida en el proyecto nuevo. Una selección que la nombre sola pide `--include-deleted`. |
| La fila es `superseded` | No vive: no choca. | Se mueve con su cadena. No toca `last_modified_at`. |
| El save tiene una pieza que otro save comparte | `pieces.rs:213-215`. | El cierre incluye los dos saves. |
| El cierre sale del proyecto de origen | Por construcción los miembros de un save y de una cadena comparten proyecto (los escribe un mismo evento y `SCOPE_ROWS_SQL` los reúne por ámbito, `topic.rs:83-86`); sólo un arreglo a mano los separa. | Rechazar y reportar: no se arrastran filas de un tercer proyecto. |
| El save tiene una pieza de otro agente | [`specs/pieces`](../../../specs/pieces/spec.md) R6 la trata como faltante bajo `?agent=`. | Con alcance de agente se rechaza (partiría el save); sin alcance, se mueve. |
| El destino no está registrado, o es el origen | Nadie lo comprueba. | 404 o 400 sin escribir; origen igual a destino es un `unchanged`. |
| La fila es una `user_preference` o una `architectural_decision` | Sacarla de su tipo pide `user_answered` (`admin.rs:266-269`, [`specs/update`](../../../specs/update/spec.md) R1). | Sacarla de un proyecto por la puerta de un agente pide lo mismo ([decisión abierta 6](#decisiones-abiertas)). |

## Lo que ya existe y se reutiliza

- Los dos chequeos de choque de `restore`: `RESTORE_TWIN` y `RESTORE_TOPIC_HOLDER` (`admin.rs:468-486`) ya expresan «gemelo vivo» y «dueña de la clave» en un ámbito; el segundo excluye además las piezas del propio save (`:483-484`) y lo que el borrado del propio save revivió (`:482`). Les falta recibir el proyecto destino en lugar del de la fila.
- El libro de lo ya colocado del import (`Ledger`, `import.rs:226-234`), que hace que las filas de un mismo archivo se vean entre sí: un lote de movimientos necesita lo mismo, porque dos saves seleccionados pueden chocar uno con otro en el destino.
- La forma de un 409 por choque, con su mensaje que nombra el id (`RestoreError::LiveTwin` y `TopicTaken`, `admin.rs:414-417`; `api.rs:1537-1539`).
- `NeedsUser` y `MemoryPatch::answered` para las filas protegidas (`admin.rs:185-200`, `:266-269`).
- `register_project` y `RegisterError::RootCollision` para registrar el destino por su ruta y fallar cerrado ante un id ajeno (`project.rs:64-150`); `ProjectIdentity::resolve` en el comando, como `projects add` (`crates/clawg-cli/src/projects.rs:23-31`), y `project_for` en el puente (`server.rs:700-733`).
- La caja de snapshots: `Reason`, `Reason::ALL`, `is_automatic` (`crates/clawg-core/src/memory/backup.rs:127-159`) y la poda que conserva el último de cada razón (`backup.rs:274-278`), de modo que un `pre-move` no empuja fuera al `pre-migration`. El precedente de tomar el snapshot antes de escribir es `import.rs:586`.
- `update_memory_core` ya lee y escribe una fila en una transacción de la conexión única (`admin.rs:254-283`); el movimiento se compone en la misma, de modo que un cambio de importancia y de proyecto juntos son atómicos.
- `Scope` y `Filters` del CLI (`crates/clawg-cli/src/cli.rs:524-565`) para el comando masivo, y `ListParams` (`admin.rs:807-822`) para vista previa.
- Para la prueba de datos previos: `a_database_from_the_previous_version_opens_with_an_empty_outbox` y compañeras (`crates/clawg-core/src/memory/db.rs:965`).
- Un movimiento no necesita el embedder (no vectoriza ni rehace el índice de texto), así que, a diferencia de `restore` e `import` (`api.rs:1532`, `:1675`), funciona mientras el modelo carga.

## Enfoques posibles

### A. Un motor en `clawg-core` con dos puertas (elegido)

Un módulo nuevo con una función pura que, dada la selección, el estado leído y el destino, devuelve el plan (qué se mueve, qué se arrastra, qué se rechaza y por qué), y otra que lo aplica en **una** transacción de la conexión única: cambia `project_id` y, si hace falta, `scope`, y anota una fila por cada una en un registro propio. Sin embeber, sin detector, sin tocar texto.

- **A1. Puerta de un save.** `PATCH /v1/memories/memory/{id}` acepta `project_id` o `no_project: true` (no ambos), solo o junto con `importance` y `memory_type`, todo en la misma transacción. El puente lo expone en `update_memory`. Un choque es un 409. La respuesta suma el proyecto en que quedó (`UpdatedRow`, `admin.rs:203-211`).
- **A2. Puerta masiva.** `POST /v1/memories/move` con una selección por filtros o ids, `dry_run` y el mismo destino; el CLI la usa en `clawg projects move`. La selección se resuelve en el daemon, en una transacción, y devuelve un reporte (qué se movió, qué se arrastró, qué se omitió y por qué, pares que quedan entre proyectos, snapshot, id del movimiento). Un choque omite ese save y sigue. Toma un snapshot `pre-move` antes de escribir, como el import.
- Los dos caminos usan el mismo plan: un `--dry-run` y un movimiento real dicen lo mismo, como en el import (`specs/portability` R7).

### B. Sólo la puerta de un save

Cubre al agente que se equivoca de proyecto, no la importación de 44: 1.594 notas, por ids, a mano. Se descarta como único camino y queda como primer corte posible (ver «Alcance»).

### C. Re-importar con otro mapa de proyectos y que mueva lo reconocido por origen

No necesita guardar el nombre de proyecto de la fuente si el archivo sigue a mano, pero obliga a conservar el export entero (que 44 pide no dejar en el repo, por privacidad), no cubre las notas sin proyecto (no hay nombre que mapear) y mezcla dos cosas: traducir una fuente y acomodar lo ya guardado. Descartado como camino, aunque 44 puede apoyarse en el motor más adelante para ofrecerlo.

### D. SQL a mano con el daemon parado (lo de hoy)

No cierra el save, no mira choques, no deja registro y exige parar el daemon. Es la situación que se quiere reemplazar.

## Con 76 antes y con 77 antes

76 todavía no tiene carpeta; se asume su planteo: `agent_id` pasa a ser sólo autoría, cualquier agente lee y cambia cualquier fila, y el dedup, el detector, la revisión por `topic_key` y `synthesize` se acotan por proyecto en lugar de agente más proyecto.

| Aspecto | Con 76 (el modelo asumido) | Si 77 llega primero |
|---|---|---|
| Quién mueve por la puerta de un save | Cualquier agente, cualquier fila. | Sólo sus filas (`?agent=`, `admin.rs:213-214`); las de `migrated` sólo el comando del usuario. |
| Ámbito de un choque | Todas las filas vivas del destino. | Las del mismo agente en el destino (`t.agent_id IS m.agent_id`, `admin.rs:469`, `:480`): dos agentes pueden tener hoy el mismo texto o la misma clave vivos en un proyecto, y no chocan. |
| Ámbito de la cadena de revisiones | Proyecto y clave (`revival.rs:66-68` cambia). | Agente, proyecto y clave. |
| Save con piezas de dos agentes | Se mueve entero. | Con alcance de agente se rechaza; sin alcance, se mueve. |
| Dos saves del mismo lote, de agentes distintos, con el mismo texto o clave | Chocan entre sí en el destino: el plan los ve uno al otro, como el import ve su libro (`import.rs:305-308`), y mueve el primero. | No chocan: cada agente tiene su ámbito. |
| `synthesize` | Por proyecto: la fila movida figura en el `pending` de B para todos. | Por agente y proyecto. |
| Lo que no cambia | Qué se mueve, el cierre, el registro, el snapshot, el visor, el CLI, la visibilidad de lo sin proyecto. | Idem. |

El predicado «mismo ámbito» está escrito a mano en `topic.rs:83-86`, `store_insert.rs:300-307`, `admin.rs:468-486`, `import.rs:236-250`, `revival.rs:66-80`, `inspect.rs:240-243` y `inspect.rs:293-298`. Este cambio suma una copia más, la del destino. Si 76 llega antes y ya lo centralizó, 77 lo usa; si llega después, 76 tiene un sitio más que cambiar, que la prueba de choques por ámbito (ver «Cómo se mide») le va a señalar.

## Decisiones tomadas

- **Lo sin proyecto se sigue viendo desde todos los proyectos** (`search.rs:95`). Nada se restringe; el cambio sólo permite reubicar. Pasar una fila a «sin proyecto» la hace visible en todos, y el reporte lo dice.
- **Se diseña sobre el modelo de 76** y se dice qué cambia si 77 llega antes (tabla de arriba).

Las resolvió la evidencia:

- **El comando del usuario es el camino de lo importado.** Las filas del agente `migrated` de 44 las mueve el comando sin alcance de agente, que vale sobre cualquier fila (`specs/visibility` R4, «sin scope»); por la puerta de un agente sólo las moverá 76.
- **Sólo `memory`.** Es la única tabla que escribe el producto (`admin.rs:399-403`) y la única que viaja en un export. Mover `episodic` o `procedural` responde 400, como `restore` (`RestoreError::OnlyMemory`, `admin.rs:408`).
- **La unidad es el save.** Piezas, saves que comparten una fila, cadena de revisión de la fila (misma clave, causa `revision`) y hermanas borradas se mueven juntos. Una fila borrada de un save que se mueve tiene que ir con él, o un `undelete` posterior la devolvería al proyecto viejo.
- **Una sola escritura de datos: `project_id`, y `scope` cuando el CHECK lo pide.** No se toca `last_modified_at` ni `update_count`: `synthesis.rs:311-315` usa `last_modified_at` de una fuente reemplazada como el momento en que se perdió, y moverla no es perderla. El momento del movimiento queda en el registro.
- **Un movimiento no es una afirmación nueva.** `created_at` se conserva, así que en un choque de clave no gana «la más nueva»: la fila que sostiene hoy la clave en el destino es la vigente, y la movida no la desplaza.
- **El destino tiene que estar en `project`.** El daemon lo comprueba; el puente y el comando lo registran por su ruta (sin `index_docs`, `project.rs:92-102`).
- **`project_rekey` no se reutiliza como registro** (`NOT NULL`, `V9__project.sql:21-22`); hace falta una tabla propia, con su migración y su prueba de datos previos.
- **Sin vectores ni índice de texto**, y por lo tanto sin embedder: el movimiento tiene que funcionar con el modelo cargando.
- **Los pares y las derivaciones no se mueven con el save:** son cercanía y procedencia, no identidad. El reporte los nombra.

## Decisiones abiertas

- [ ] **1. Qué hacer ante un choque de clave o de texto en el destino.**
  - (a) Rechazar el save (409 en una fila; en el lote se omite y se informa con el id que lo sostiene). Resolver a mano: `delete` de una y repetir.
  - (b) `history`: la movida entra `superseded` por la vigente, causa `revision`, como hace el import (`import.rs:314-316`). Es la única causa que `revival` revive y exige el mismo ámbito (`revival.rs:66-68`), que tras el movimiento se cumple: borrar la vigente del destino devuelve la movida. Con causa `verdict` quedaría escondida hasta que otro veredicto la devuelva.
  - (c) La movida reemplaza a la vigente (`revise_in`, `topic.rs:111`).
  - (d) Para un texto igual, fundir: sobrevive la más vieja (`REINFORCE_LOOKUP_SQL` ordena por `created_at ASC`, `store_insert.rs:307`), suma accesos y la otra se borra suave.
  - Costos: (b), (c) y (d) cambian `status`, `superseded_by` o `deleted_at`, y (c) y (d) esconden una fila viva, mientras que el registro de vuelta atrás tendría que cubrir esos cambios (hoy ni `project_rekey` lo hace, y 30 lo anota como su problema). Con (a) lo único que cambia es `project_id` y la vuelta es exacta.
  - **Recomendación:** (a) en el primer corte. (b) se suma si la corrida de prueba en `dev` muestra que los choques de lo importado importan; en ese caso comparte el motor de reconciliación con 30.
- [ ] **2. Unidad del movimiento.**
  - (a) El cierre: save, saves que comparten fila, cadena de revisión y hermanas borradas (elegido arriba como base).
  - (b) Rechazar toda selección que parta una unidad.
  - (c) Mover sólo lo seleccionado.
  - (c) parte saves y rompe `revival`; (b) obliga a seleccionar a mano la cadena entera; (a) arrastra filas que nadie nombró, y por eso el `--dry-run` las lista como `pulled_in` y el cierre que cruza de proyecto de origen se rechaza.
  - **Recomendación:** (a).
- [ ] **3. Vuelta atrás.**
  - (a) Sólo el snapshot `pre-move`: se vuelve con `clawg restore`, que descarta todo lo guardado después.
  - (b) Sólo un registro por fila (`memory_move`: id del movimiento, fila, proyecto y `scope` de origen y de destino, momento).
  - (c) Los dos, más `clawg projects move --undo <ID>` que devuelve las filas que siguen en el destino registrado (la condición de `V9__project.sql:12-16`).
  - Costos de (b) y (c): una migración y su prueba de una base de la versión anterior, que `config.yaml` exige para todo cambio de identificadores guardados; en (c), un `Reason::PreMove` (`backup.rs:127-159`) y su entrada en los listados de razones.
  - **Recomendación:** (c): registro siempre (también el `PATCH`), snapshot sólo en el lote, como el import.
- [ ] **4. Puerta para los agentes.**
  - (a) Ampliar `update_memory` con un destino (`to_project_path`) y `clear_project`.
  - (b) Una herramienta nueva `move_memory`.
  - (c) Ninguna: sólo el comando.
  - (a) cuesta lo menos en contexto y vive junto a las otras correcciones de metadatos, pero su ausencia tiene que significar «no cambiar», al revés que `project_path` en las demás herramientas, donde ausente es el proyecto del puente (`resolve_project`, `crates/clawg-cli/src/mcp/tools.rs:557-570`): el nombre tiene que ser otro. (b) agrega una herramienta al contexto de cada sesión y cambia la lista de doce. (c) deja al modelo sin poder corregir su propio save en el proyecto equivocado.
  - Cualquiera de (a) o (b) cambia `agent_contract.json` (`CLAWG_BLESS=1 cargo test -p clawg-cli the_agent_contract_matches_its_file`) y pide una línea en `protocol.md`.
  - **Recomendación:** (a). La puerta masiva no se ofrece a un agente.
- [ ] **5. Dónde vive el comando masivo.**
  - (a) `clawg projects move`; (b) `clawg memories move`; (c) un comando de primer nivel.
  - `memories` se documenta de sólo lectura (`cli.rs:202-203`, `.claude/CLAUDE.md`): una regla de permisos que le deje al modelo navegar con `clawg memories` pasaría a dejarle reescribir. `projects` ya muta (`add`).
  - **Recomendación:** (a), con `--to PATH` (se resuelve y registra como un puente), `--to-project ID` (para una raíz que ya no existe en disco) y `--to-none`.
- [ ] **6. Filas protegidas.**
  - (a) Sacar de un proyecto una `architectural_decision` o una `user_preference` por la puerta de un agente pide `user_answered` (409 `NeedsUser`).
  - (b) No pedirlo.
  - Una decisión que sale de A sale del AUTO de A (`workspace.rs:281-291`) y de su pool de disputas (`contradiction.rs:172-184`): hacerlo sin la palabra del usuario es la baja silenciosa que la guarda de `specs/update` R1 existe para frenar. El comando del usuario lo declara por construcción.
  - **Recomendación:** (a).
- [ ] **7. Qué es un destino válido.**
  - (a) Un id presente en `project` (el daemon lo exige); el puente y el comando piden además que la ruta exista o que el id ya esté registrado.
  - (b) Cualquier id.
  - Con (b) un error de tipeo esconde la fila y `doctor` no lo ve (`checks.rs:372-392`); guardar con una ruta inexistente hoy sólo la registra como etiqueta ([`specs/project`](../../../specs/project/spec.md) R6), pero un save equivocado se ve en el proyecto de la sesión, y una fila movida no.
  - **Recomendación:** (a). Un chequeo de `doctor` para filas bajo un id que no está en `project` queda como candidato, a medir antes: los ids de antes del registro de raíces pueden darle ruido.
- [ ] **8. Detector al mover.**
  - (a) No correrlo, como el import ([`specs/portability`](../../../specs/portability/spec.md) R3).
  - (b) Correrlo contra el destino con el vector guardado, como hace `restore` (`admin.rs:510-540`).
  - (b) escribe pares y puede dejar una fila `disputed`: más estado que deshacer. Con (a) una decisión movida puede contradecir a otra del destino sin que nadie lo sepa hasta que un save cercano la encuentre.
  - **Recomendación:** (a), y que el reporte cuente cuántas filas protegidas llegaron al destino. (b) como cambio posterior si la corrida en `dev` muestra el problema.
- [ ] **9. Dónde guarda 44 el nombre de proyecto de la fuente.** Ver la sección siguiente.
- [ ] **10. Qué filtros tiene la selección masiva.** Mínimo para el motivo: `--ids`, `--from-project ID`, `--from-none`, `--here`, `--agent`, `--type`, `--status`, `--text` (la búsqueda léxica del visor, `ListParams::query`), `--source import`, `--include-deleted`; y, si 44 los guarda, `--origin-project NOMBRE` y `--session ID`. Al menos uno es obligatorio. Los mismos filtros, más «sin proyecto», pasan a `clawg memories list` y a `GET /v1/memories`, que hoy no pueden listar lo que hay que acomodar. **Recomendación:** ese conjunto.

## Lo que 77 necesita de 44

Requisito cruzado. Para mover en bloque por el nombre de proyecto de la fuente, ese nombre tiene que estar en el store. 44 hoy no lo guarda ([propuesta](../../history-import/proposal.md), «Decisiones»: un nombre sin mapa entra sin proyecto «y se acomoda después»), y sin él las 21 etiquetas de las 2.118 notas que sí traen proyecto se vuelven indistinguibles una vez importadas.

- **Guardar el nombre de proyecto de la fuente** en toda nota importada que lo traiga, también cuando el mapa la lleva a un proyecto (sirve para auditar y para rehacer un mapa equivocado). Opciones:
  - **(a) Columna nullable nueva** (nombre a fijar, p. ej. `source_project`), escrita sólo por los importadores. Se puede filtrar e indexar, la lleva el export (el formato ignora campos que no conoce, [`specs/portability`](../../../specs/portability/spec.md) R6) y sirve a cualquier fuente. Cuesta una migración con su prueba de datos previos, y contradice el «sin cambio de esquema» de 44.
  - **(b) Dentro de `created_by_ref`**, con un formato documentado (`<fuente>:<id>` más el nombre). Sin migración, pero un solo campo hace de clave de reconocimiento por origen y de filtro, el nombre puede traer cualquier carácter y el filtro sería un `LIKE`.
  - **(c) No guardarlo** y mover re-importando con un mapa nuevo (enfoque C): exige conservar el export y no cubre nada de lo que no trae nombre.
  - **Recomendación:** (a). Si 44 no quiere esquema, (b) con el nombre **después** del id (el reconocimiento por origen compara el prefijo) y con el formato fijado por una prueba.
- **Guardar la sesión de la fuente en `source_session_id`**, que ya existe (`V1__init.sql:16`), se muestra (`memories.rs:226`), viaja en el export (`export.rs:222`, `import.rs:329`) y ningún código usa para decidir. Es la única agrupación que les queda a las 1.594 notas sin nombre de proyecto en ningún lado: las notas de una misma sesión se mueven juntas. Hay que confirmar la forma de esos ids antes de mezclarlos con los de las sesiones del anfitrión, que el daemon valida de 1 a 128 caracteres ASCII visibles (`POST /v1/sessions`).
- **Una nota sin proyecto no lleva nada** en ese campo (NULL), nunca un marcador como `unknown`: un valor inventado se volvería un nombre de proyecto en los filtros.
- **Se mantiene** la regla de 44 de que un segundo import reconoce por origen y no mueve: después de acomodar, esa regla es lo único que evita duplicar lo importado (ver el último renglón de las premisas).
- **44 no depende de 77** para entregarse: con el nombre guardado, 77 lo aprovecha cuando llegue.

## Alcance candidato

1. Un módulo nuevo en `clawg-core` con el plan puro (selección, cierre, choques, `scope`) y su aplicación en una transacción, con el registro por fila.
2. Migración con la tabla del registro y su prueba de una base de la versión anterior; `Reason::PreMove`.
3. `PATCH /v1/memories/memory/{id}` con `project_id` o `no_project`, 409/404/400, y el proyecto en la respuesta.
4. Filtro «sin proyecto» (y los de origen cuando 44 los guarde) en `ListParams`, `GET /v1/memories` y `clawg memories list`.
5. `POST /v1/memories/move` con `dry_run`, el reporte y el snapshot; `clawg projects move [filtros] (--to PATH | --to-project ID | --to-none) [--dry-run] [--undo ID] [--include-deleted]`.
6. `update_memory` con destino y `clear_project`, `agent_contract.json`, una línea en `protocol.md` y la palabra del usuario para las protegidas.
7. Documentación: `.claude/CLAUDE.md` (el comando, la ruta y los campos) y deltas de `specs/update`, `specs/project`, `specs/mcp` y `specs/visibility`.
8. Opcional, a medir: un chequeo de `doctor` para filas bajo un id que no está en `project`.

Partido en dos: **77a** = 1, 2 (sólo el registro), 3, 6 y el filtro sin proyecto; **77b** = 5, el snapshot, `--undo` y 7.

### No-alcance

- Cambiar quién escribió una fila: es 76.
- La identidad de un proyecto, los clones y `clawg projects merge`: es 30. `merge` es el caso de mover todo un id a otro, y conviene que se construya sobre este motor en lugar de escribir otro mover.
- `episodic_trajectory` y `procedural_skill`.
- Reconciliar el choque (`history`, reemplazar, fundir) y correr el detector al mover.
- Mover la sección USER de un workspace y el índice de docs.
- Cambiar el texto, la importancia o el tipo de lo movido, o restringir lo que se ve sin proyecto.
- Que un import rehaga el mapa y mueva lo ya importado (enfoque C).
- Una puerta masiva para los agentes.
- Cambios de ranking: no se corre `rag-eval`.

## Cómo se mide

Tests RED primero. Hoy fallan todos: no hay operación (un campo de proyecto en el `PATCH` se rechaza, `ListParams` no tiene «sin proyecto», el comando no existe). Los nombres son los candidatos.

Núcleo, sobre `MemoryStore::new_for_test()` y sin embedder:

1. `a_row_without_a_project_moves_into_one` y `moving_to_no_project_shows_the_row_from_every_project`: búsqueda densa, léxica y por identificador desde A, desde B y con `all_projects`, antes y después.
2. `a_row_without_an_agent_keeps_the_scope_check`: de `project` a sin proyecto pasa a `global`, y al revés.
3. `the_pieces_of_a_save_move_together` y `two_saves_that_share_a_row_move_together`.
4. `a_revision_chain_moves_with_its_head_and_deleting_the_head_still_revives`.
5. `a_live_row_of_the_same_text_in_the_target_refuses_the_move` y `a_live_holder_of_the_topic_key_in_the_target_refuses_the_move`: nombran el id y no cambian ni filas ni registro; además, un test de choque por ámbito que 76 tenga que actualizar si cambia el predicado.
6. `a_deleted_row_moves_with_its_save_and_is_restored_into_the_new_project`.
7. `moving_a_superseded_source_does_not_make_its_conclusions_reappear_for_review`: `conclusions_to_review_count` igual antes y después.
8. `after_a_move_a_save_of_the_same_text_reinforces_the_moved_row` y `after_a_move_a_save_under_its_key_revises_it`; y `two_selected_saves_that_would_collide_in_the_target_move_only_the_first`.
9. `a_session_summary_moved_into_a_project_is_its_last_session`.
10. `a_dry_run_reports_what_a_move_does_and_writes_nothing`, con el mismo reporte que el movimiento real.
11. `every_moved_row_is_logged_and_the_log_undoes_it`, sin tocar una fila que se movió otra vez después.
12. `an_unregistered_target_is_refused` y `a_target_whose_id_belongs_to_another_root_is_a_conflict`.
13. `a_protected_row_leaves_its_project_only_on_the_users_word`.
14. `a_move_leaves_vectors_and_the_text_index_alone`, y que `clawg doctor` no gane ningún hallazgo (`memory.topic`, `memory.content_hash`).
15. `a_database_from_the_previous_version_opens_with_an_empty_move_log` (el registro es nuevo, `config.yaml`).

Daemon, puente y comando:

16. HTTP (`crates/clawg-cli/tests/api.rs`): el `PATCH` con proyecto mueve el save entero; un choque es 409; `project_id` y `no_project` juntos son 400; el movimiento funciona con el embedder cargando; `GET /v1/memories` lista las filas sin proyecto.
17. Puente: `update_memory` con destino mueve el save; no mueve filas de otro agente (con 76, sí); el contrato (`agent_contract.json`) cambia sólo por lo previsto.
18. Comando: el análisis de argumentos es puro (en `cli.rs`); en `binary.rs`, `projects move --dry-run`, el movimiento y el `--undo` sobre un store sembrado, con el snapshot `pre-move` en el lote.

A mano y con autorización, sobre el perfil `dev` y después de importar con 44: `clawg memories stats` para ver el grupo sin proyecto, `projects move --dry-run`, revisar el reporte, mover, comprobar que una búsqueda desde el destino alcanza lo movido y desde otro proyecto ya no, medir cuánto tarda el lote en la conexión única del daemon (se estima del orden de milisegundos a segundos, sin embeber: falta medirlo) y revertir con `--undo`. `rag-eval` no aplica: no se toca el ranking.

## Riesgos

- **Un lote grande en la única conexión de SQLite del daemon** (read-connections, 57). Es una transacción de `UPDATE` y de inserciones en el registro, sin vectores; se mide en `dev`. Si pesa, se parte en tandas (como el import, `import.rs:41`), a costa de una aplicación parcial que el registro cubre.
- **Pasar una fila a «sin proyecto» la hace visible desde todos** y candidata del detector de todos. Está decidido que así sea; el reporte dice cuántas filas se vuelven visibles.
- **Volver a importar un export de clawg después de mover duplica** (último renglón de las premisas). Sólo el reconocimiento por origen de 44 lo evita, y sólo para lo que viene de otra herramienta. Hay que documentarlo.
- **Un cierre que arrastra más de lo esperado.** El `--dry-run` lo lista (`pulled_in`) y un cierre que cruza de proyecto de origen se rechaza.
- **Dos escritores de `project_id`.** El SQL de vuelta del re-key (`V9__project.sql:12-16`) mueve las filas que hoy están en el proyecto que anotó; una fila re-keyada de A a B y movida después de B a C no se toca, pero una movida de C a B la devolvería a A. Hay que documentarlo junto a la vuelta atrás de cada uno.
- **Un destino que es un fantasma** (error de tipeo en una ruta del modelo). Lo frena exigir que esté registrado o que el directorio exista (decisión abierta 7).
- **Contradicciones que llegan sin que nadie las vea** (decisión abierta 8): el movimiento no corre el detector.
- **Ruido en el destino.** Mover miles de filas a un proyecto agranda su pool de KNN; el ranking no cambia y no se corre `rag-eval`, pero la búsqueda real en `dev` lo muestra (lo mismo que ya anota 44).
- **Una migración más.** 30, 44 y 76 también pueden agregar una; el número lo fija quien aterrice después, y la prueba de datos previos de cada una parte de la anterior.
- **Dos movers.** Si `projects merge` (30) se escribe aparte, hay dos caminos que cambian `project_id` con reglas de choque distintas.
