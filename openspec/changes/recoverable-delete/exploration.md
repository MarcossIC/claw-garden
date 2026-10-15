# recoverable-delete — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | M | — (todo lo previo está archivado) | cerrada: sigue en la [propuesta](proposal.md) |

**Tesis:** borrar una memoria es un error que hoy no se puede deshacer, y es el único que queda sin salida. El texto sigue en la base, pero la fila quedó fuera del índice de búsqueda, del índice léxico y de la tabla de vectores. Nadie sabe volver a ponerla. Es la parte E de data-preservation (`openspec/changes/archive/2026-10-12-data-preservation/exploration.md:96`), que se archivó sin cambio propio.

## Revisión contra main (2026-10-15)

Revisada contra `main` (9aa1012), después de que se archivaran long-save-context, agent-sharing y project-indexing. Las líneas citadas abajo son de 2026-10-14; las de la propuesta, la spec y el diseño son las de hoy.

| Lo de abajo | Hoy |
|---|---|
| "Un chunk es una fila. Restaurar no re-trocea nada: es una fila, un texto, un vector" | Sigue siendo una fila, pero un pedazo de un save largo se embebió detrás de la cabecera del save, guardada en `memory_piece.context` (V11, [`specs/pieces`](../../specs/pieces/spec.md)). Restaurarlo tiene que usar ese texto, como el re-embed (`maintenance.rs:635-652`). `read_memory` nombra el pedazo borrado como faltante. |
| Nueve herramientas MCP (`server.rs:1361`) | Son once (`read_memory` y `search_project` se sumaron): `restore_memory` sería la duodécima (`server.rs:1520`, `handshake.rs:442`). |
| Conflicto 9: "¿el id purgado se reusa?" | **Resuelto:** `specs/synthesis` R6 ya dice que no se vuelve a usar, por `AUTOINCREMENT`. |
| Conflicto 5: `superseded` | Sigue, y ahora es real: `resolve_conflict` escribe `superseded` (`dispute.rs:192`). |
| Scope de filas compartidas | agent-sharing se archivó con "todo compartido": un agente ve lo de otros, pero borrar, cambiar y resolver siguen siendo de su autor (`specs/sharing` R5). Restaurar va con la misma regla. Además, un secreto en una fila restaurada viaja a otros agentes por la búsqueda: es más razón para rechazarlo. |
| Embedding | El contrato pasó a mean pooling (project-indexing). No cambia nada: el restore embebe con el embedder vigente. |
| `admin.rs` | `delete_memory_core` está en `admin.rs:300-348`; el listado filtra en `admin.rs:633, 697, 752`; `update_memory` (antes `PATCH` de importancia) contesta 404 a una borrada (`admin.rs:189`). |
| Backlog: `purge` deja el vínculo de pedazo | No afecta: un id purgado no se restaura. |

## Qué dice el texto de 2026-10-12 y qué ya no es cierto

| Afirmación de entonces | Hoy |
|---|---|
| El vector se borra en `admin.rs:211-213` | Es `admin.rs:330` (`delete_memory_core`, `admin.rs:300-348`). La función además libera las disputas que la fila sostenía (`admin.rs:334-336`). |
| "El delete no tiene scope: cualquier agente borra cualquier memoria" (riesgo 5) | **Ya no es cierto.** `delete_memory_core` recibe `owner` y no escribe nada si la fila es de otro agente (`admin.rs:324-328`). El MCP siempre manda el agente que llama (`crates/clawg-cli/src/mcp/server.rs:602-615`). Contrato en `specs/visibility` R3 y R4. |
| "No hay purga" y la decisión abierta "restaurables siempre, o papelera con purga" | **Ya no es cierto.** Existe `clawg purge <ID> --yes` (`specs/secrets` R5, `crates/clawg-core/src/memory/secrets.rs:81`), sólo por CLI, con el daemon parado. La mitad de la decisión ya se tomó en secret-hygiene: el delete sigue siendo lógico y la purga es otro comando. Queda abierto sólo si la papelera tiene retención (ver decisiones). |
| "No hay backup ni restore" (riesgo 3) | Ya existen (`specs/storage`). `clawg restore <archivo>` reemplaza **toda** la base. No sirve para recuperar una fila: pierde todo lo guardado después del snapshot. |
| "No hay borrado por proyecto"; depende de project-identity | Sigue sin existir. project-identity ya está archivado: el id sale de la raíz del repo (`specs/project`) y hay tablas `project` y `project_rekey` (`crates/clawg-core/migrations/V9__project.sql`). |
| "Poner `deleted_at = NULL` a mano no alcanza: falta el vector y `reembed_all` sólo recorre filas con vector" | **Sigue siendo cierto**, y además falta la entrada FTS (abajo). `reembed_all` filtra `deleted_at IS NULL` y hace `JOIN` con la tabla de vectores (`crates/clawg-core/src/memory/maintenance.rs:618`, `:643`). |
| `DELETE /v1/memories/{kind}/{id}` en `api.rs:347-349` | Es la ruta de `api.rs:538-544` y el handler de `api.rs:914-928`. |

## Problema (verificado contra el código de hoy)

### Qué hace un delete, tabla por tabla

| | `memory` | `episodic_trajectory` | `procedural_skill` |
|---|---|---|---|
| Marca | `deleted_at = ahora` (ms), sólo si estaba NULL (`admin.rs:271`) | igual (`admin.rs:272-273`) | igual (`admin.rs:274-275`) |
| Vector | se borra en la misma transacción, siempre, aunque el `UPDATE` no toque filas (`admin.rs:281-283`, `:330`) | igual | igual |
| Índice FTS | el trigger `memory_fts_au_deleted` resta los términos en la transición viva a borrada (`V8__memory_fts.sql:73-78`) | no tiene FTS | no tiene FTS |
| Disputas | las filas `disputed` contra esta vuelven a su estado de reposo (`release_challengers`, `dispute.rs:132`; llamado en `admin.rs:334-336`) | no aplica | no aplica |
| Texto, `content_hash`, importancia, tier, `status`, contadores | quedan tal cual | quedan | quedan |
| Procedencia (`memory_derivation`) | queda (`specs/synthesis` R6) | no tiene | no tiene |

Dos detalles que importan:

- **Un chunk es una fila.** El troceo ocurre antes de insertar (`worker.rs:505`, `store_insert.rs:327`). Restaurar no re-trocea nada: es una fila, un texto, un vector.
- **El decaimiento no salta las filas borradas.** `demote_sql` no filtra `deleted_at` (`maintenance.rs:41-47`). Una fila que lleva meses borrada vuelve con el tier más bajo que le tocaba, y con `last_accessed_at` viejo.

### Caminos de borrado

- MCP `delete_memory` → `DELETE /v1/memories/{kind}/{id}?agent=` → `delete_memory_core` (`server.rs:765-776`, `api.rs:914-928`). Responde `deleted 1`, o 404 si no existe, ya estaba borrada o es de otro agente.
- No hay otro camino. Las filas **no se borran físicamente** salvo con `clawg purge`.
- En producción sólo se crean filas de `memory`. `insert_episodic_trajectory` y `promote_to_procedural_skill` sólo se llaman desde tests y desde el armado de prompt de tests (`rg` sobre `crates`). Las tres tablas existen y el delete las cubre, pero hoy una base real sólo tiene borradas de `memory`.

### Por qué no se puede deshacer

- No hay función que restaure. `rg` por `restore` en `crates/clawg-core/src/memory/` sólo encuentra el restore de snapshots.
- El comentario de V8 lo dice: el trigger inverso no existe "porque ningún camino de código recupera una memoria" (`V8__memory_fts.sql:70-72`).
- `PATCH` no sirve: sólo toca filas vivas y las borradas contestan 404 (`admin.rs:188-193`; test `update_on_a_soft_deleted_row_changes_nothing`).
- Un `save_memory` del mismo texto **no** lo resucita: la consulta de dedup excluye las borradas (`store_insert.rs:236-243`), así que crea una fila nueva. El dato vuelve, pero con otro id, y sin su historia (importancia, contadores, procedencia).

### Cómo se encuentra una fila borrada

- `list_memories` (MCP) y `GET /v1/memories` **nunca** muestran borradas: todos los `SELECT` de listado llevan `deleted_at IS NULL` (`admin.rs:633`, `:697`, `:752`; doc en `admin.rs:500`).
- `GET /v1/memories/{kind}/{id}` sí devuelve una borrada, con `deleted_at` (`admin.rs:584-588`, `api.rs:881`). Sirve si ya se tiene el id. No lleva scope de agente, y el MCP no lo expone.
- `clawg memories show` marca `[deleted]` (`crates/clawg-cli/src/memories.rs:166`). `clawg memories stats` cuenta cuántas hay (`memories.rs:227`). Nada las lista.
- `clawg secrets` lista las borradas **que tienen un secreto** (`openspec/specs/secrets/spec.md:92`).
- Dentro de la misma sesión el agente tiene el id: lo acaba de pasar a `delete_memory`. Entre sesiones, no hay forma sin SQL.

## Lo que ya existe y se reutiliza

- `to_unit` + `serialize_f32_vec` y el `INSERT INTO memory_vectors(rowid, embedding)` de `insert_memory_with_vector` (`store_insert.rs:327-372`).
- Embeber una fila suelta: `Embedder::embed_with_role(texto, EmbedRole::Passage)`. Es lo que ya hace `reembed_all` por fila (`maintenance.rs:577-591`). No hace falta el pipeline de ingest. El de ingest troza, deduplica y reinserta como fila nueva.
- El texto que se embebe, por tabla: `memory.content`, `episodic_trajectory.task` y `skill_embedding_text(trigger, skill_name)` (`maintenance.rs:359`, `:636-638`).
- `redact(&str).kinds` es pura (`crates/clawg-core/src/memory/redact.rs:131`): permite saber si el texto de una fila vieja aún tiene un secreto.
- `detect_contradiction(store, vec, agent, project, tipo, siblings)` (`crates/clawg-core/src/memory/contradiction.rs:122`) recibe un vector ya calculado.
- `settle_disputed` / `settle_sql` recalculan el estado de reposo de una fila (`dispute.rs:142`).
- El patrón de scope `?agent=` / `owner` de DELETE y PATCH (`api.rs:389`, `admin.rs:277-279`) y el error `embedder_failed` del daemon (`api.rs:123-130`).
- El embedder vive en el daemon (`runtime.rs:92`). Un restore por el daemon hereda **el contrato de embedding vigente**: el daemon sólo sirve si `ensure_embedding_contract` pasó (`bootstrap.rs:246`). Un restore fuera del daemon tendría que cargar el modelo ONNX en otro proceso.

## Qué haría falta para restaurar (y dónde está cada conflicto)

Una operación atómica `restore_memory_core(store, embedding, id, kind, owner)`. El embedding se calcula **antes** de abrir la transacción (el embedder es async y lento). Dentro de la transacción:

1. Releer la fila: debe seguir borrada y ser del agente (si hay `owner`). Si no, `NotFound`.
2. Chequear el dedup (conflicto 1).
3. `UPDATE ... SET deleted_at = NULL`, más lo que se decida sobre `last_modified_at`, `update_count` y `last_accessed_at`.
4. Borrar cualquier vector huérfano con ese rowid e insertar el nuevo. Un delete previo a la poda en línea puede haber dejado uno hasta que corra el barrido (`maintenance.rs:169`, `:100-115`). Un `INSERT` a secas fallaría.
5. Reinsertar la entrada FTS **una sola vez**. El índice de V8 es de contenido externo y subtractivo: cada fila viva está sumada exactamente una vez (`V8__memory_fts.sql:50-62`). Hay dos formas (decisión abierta).
6. Recalcular el estado de la propia fila (conflicto 3) y, si se decide, detectar contradicciones nuevas.

### Conflictos

1. **Dedup por hash.** La consulta de dedup exige `deleted_at IS NULL` (`store_insert.rs:236-243`) y el índice es parcial sobre vivas (`V6__content_hash.sql:15-16`). Si el mismo texto se guardó de nuevo después del borrado, hay una fila viva con el mismo `(agent, proyecto, hash)`. El índice no es `UNIQUE` (`V6__content_hash.sql:9-12`), así que restaurar a ciegas produce dos filas vivas iguales, justo lo que el dedup evita.
2. **Disputas.** `specs/disputes` R4 ya fijó la regla: "un undelete futuro no restaura las disputas que se liberaron al borrar" (`openspec/specs/disputes/spec.md:87`). Quedan dos cosas:
   - la fila restaurada puede tener `status = disputed` y un `disputed_against` que nombra filas que ya no están vivas. Hay que recalcular su reposo en la misma transacción. Si no, la limpieza diaria lo hace después (`maintenance.rs:210`);
   - lo que se guardó mientras estuvo borrada puede contradecirla. Hoy la contradicción se detecta sólo al ingerir (`worker.rs:544`), nunca al restaurar. Sin re-detección conviven en silencio dos hechos opuestos.
3. **Conclusiones de síntesis.** No hace falta código. `memory_derivation` no se toca al borrar (`specs/synthesis` R6) y los predicados `PENDING` y `CONCLUSION` miran `deleted_at` (`synthesis.rs:32-38`). Restaurar una conclusión vuelve a cubrir a sus fuentes. Restaurar una fuente la deja pendiente o cubierta según haya una conclusión viva. Sí hace falta **un test** que lo fije.
4. **Filas purgadas.** No se pueden restaurar: la fila, el vector, el FTS y la procedencia ya no existen (`specs/secrets` R5, paso 6). El error es el mismo 404 que un id inexistente. Decir "fue purgada" revelaría el pasado de un id ajeno, y la regla de visibility es que un id ajeno responde como inexistente. Los snapshots pueden seguir teniéndola (`specs/secrets` R6); no hay camino por fila hacia ellos.
5. **Superseded.** Una fila reemplazada y luego borrada vuelve con `status = superseded`. La búsqueda y el recall excluyen ese estado (`search.rs:119`), así que el restore "funcionaría" y la fila seguiría invisible.
6. **Secretos.** Las filas anteriores al enmascarado (`redact`) pueden tener un secreto en texto plano. Hoy, estando borrada, no se busca ni se recuerda. Restaurarla la devuelve a la búsqueda, al contexto y a `workspace.md` (`specs/secrets`).
7. **Importancia, tier y decaimiento.** Se conservan los valores al borrar, salvo que el decaimiento haya seguido bajando el tier (`maintenance.rs:41-47`). Con `last_accessed_at` viejo, la fila vuelve con poca recencia y puede no aparecer nunca más en el ranking aunque esté "viva".
8. **Cambio de modelo entre medio.** Si hubo un re-embed con otro modelo mientras estaba borrada, esa fila no se re-embebió (`maintenance.rs:618`). No es un problema: el restore embebe con el embedder vigente. Hay que fijarlo con un test para que nadie lo "optimice" reusando un vector viejo.
9. **Id reutilizado.** Las tablas son `AUTOINCREMENT` (`V1__init.sql:5`), que no reutiliza ids. Pero `specs/synthesis` R6 dice que el id purgado "puede volver a usarse". Si fuera cierto, restaurar un id actuaría sobre otra fila. Hay que verificarlo antes de la spec.

## Enfoques posibles

### E1. Restaurar por id, mismo contrato que el delete (recomendado como base)

`restore_memory_core` en `admin.rs`, simétrica a `delete_memory_core`: `(id, kind, owner)`, atómica, `NotFound` uniforme para inexistente, viva, purgada o ajena. Se expone por HTTP y CLI, y opcionalmente por MCP.

- A favor: un único camino, sin migración si el FTS se reinserta a mano, y el scope sale de lo ya probado.
- En contra: no resuelve el descubrimiento entre sesiones por sí sola.

### E2. Descubrimiento: un filtro `deleted`

El listado de borradas hace falta para que la restauración sea usable entre sesiones.

- **HTTP:** `GET /v1/memories?deleted=true`, ordenado por `deleted_at` descendente (el listado de hoy ordena por `created_at`, `admin.rs:503`). Se pone en `ListParams` y `list_params_from_query` (`api.rs:196`).
- **CLI:** `clawg memories list --deleted`. Es el visor del humano, que `server.rs:1366-1369` mantiene fuera del alcance del modelo.
- **MCP:** NO. `list_memories` tiene un test que fija sus cinco argumentos (`the_viewer_is_not_exposed_to_the_model`, `server.rs:1369`). Agregarle `deleted` abre otra decisión.

### E3. Restauración manual con SQL

No es un enfoque: reproduce el error de dejar la fila sin vector ni FTS. Se descarta.

## Superficie: tradeoffs

| Opción | Costo en el contexto del agente | Qué se puede descubrir y deshacer | Notas |
|---|---|---|---|
| **A. Herramienta MCP `restore_memory`** (la décima) | Una descripción y un schema más en cada sesión. El repo no mide ese costo; `context-density` trabajó sobre el texto de `recall_context`, no sobre las herramientas | El agente deshace su propio borrado en la misma sesión (tiene el id). Con scope, sólo lo propio | Rompe tests que fijan nueve herramientas (`server.rs:1361`, `:1372-1384`) y `specs/mcp` (`openspec/specs/mcp/spec.md:43`, `:56`, `:62`). Hay que cambiarlos a propósito. |
| **B. Campo en `update_memory`** (`deleted: false`) | Cero herramientas nuevas | Igual que A | Mezcla dos contratos. `update_memory` corrige importancia o tipo, exige uno de los dos (`MemoryPatch::new` → `Empty`, `admin.rs:160`) y contesta 404 para borradas (`specs/update`). La descripción se hace más larga igual. No se recomienda. |
| **C. Sólo HTTP y CLI** | Cero | El usuario deshace. El agente no | Coherente con "lo irreversible lo decide el usuario" (`purge`), pero restaurar **es** reversible (se vuelve a borrar). La asimetría no tiene la justificación que sí tiene la purga. |
| **D. C primero, A después de medir** | Cero ahora | Igual que C hasta decidir | Es lo más barato. Retrasa el caso "me equivoqué, deshacelo" en la conversación. |

Nombres, porque hay colisiones:
- `clawg restore <archivo>` ya es el restore de snapshots (`cli.rs:73`). El comando por fila no puede llamarse `restore`. Candidatos: `clawg memories undelete <kind> <id>` o `clawg undelete`.
- `clawg memories` hoy se documenta como "solo lectura" (`memories.rs:1`, `cli.rs:104`). Un subcomando que escribe cambia esa promesa.
- HTTP: `POST /v1/memories/{kind}/{id}/restore`. `PATCH` con `deleted: false` mezclaría contratos, igual que B.

## Borrado por proyecto ("olvidar un proyecto")

No entra en este cambio. Va en uno aparte, **después** de éste, porque necesita lo de acá para poder deshacerse.

- project-identity ya está archivado: el id sale de la raíz (`specs/project`) y la tabla `project` guarda `root_path` (`V9__project.sql`). Se puede resolver "este repo" a su id, y con `project_rekey` a los ids viejos. El motivo de la dependencia de 2026-10-12 ya no existe.
- La trampa que sigue en pie: las `user_preference` guardan el `project_id` de la carpeta donde se guardaron (`store_insert.rs:179`) y se ven desde todos los proyectos por una cláusula (`search.rs:122`, `:180`, `:559`). Un borrado por `project_id` ingenuo borra preferencias globales. Hay que excluir `memory_type = 'user_preference'` y las filas sin proyecto.
- Para deshacerlo en bloque hace falta saber qué filas cayeron juntas. Un delete por fila estampa su propio `deleted_at`; uno masivo podría compartir un único sello. Eso es diseño del cambio siguiente, no de éste.
- Qué le pide este cambio al siguiente: nada más que `restore` por id. No se agrega una API en lote ahora.
- Es una operación masiva y destructiva: sólo CLI, con confirmación, como `purge`.

## Alcance candidato

1. `restore_memory_core(store, embedding, id, kind, owner)` en `crates/clawg-core/src/memory/admin.rs`: atómica, simétrica al delete, `NotFound` uniforme.
2. Reinserción del vector (borrando un huérfano si lo hay) y de la entrada FTS exactamente una vez.
3. Dedup: si hay una fila viva con el mismo `(agent, proyecto, hash)`, no restaura y nombra esa fila viva.
4. Recalcular el estado de la propia fila y, según se decida, detectar contradicciones con lo guardado después.
5. Ruta `POST /v1/memories/{kind}/{id}/restore?agent=`: 200 con la fila, 404 uniforme, 409 por dedup, 503 si el embedder no está listo.
6. Filtro `deleted=true` en `GET /v1/memories` y `clawg memories list --deleted`.
7. Comando de CLI por fila (nombre a decidir) y, según la decisión, la herramienta MCP `restore_memory` con scope por agente.
8. Docs: `CLAUDE.md` del repo (comandos, ruta HTTP y herramientas) y la descripción de `delete_memory` para que diga que se puede deshacer.

### No-alcance

- Borrado por proyecto (cambio aparte).
- Papelera con retención, vaciado automático o TTL (a menos que se decida lo contrario).
- Importar una fila desde un snapshot.
- Restaurar filas purgadas.
- Restaurar disputas liberadas (ya fijado por `specs/disputes` R4).
- Cambios de ranking, de esquema y de contrato de embedding. Con FTS reinsertado a mano no hay migración.
- Un listado de borradas para el modelo.

## Decisiones abiertas

Son del usuario.

- [ ] **Superficie.** ¿Herramienta MCP `restore_memory` (A), sólo HTTP y CLI (C), o C ahora y A después de medir (D)? Descartar B salvo razón nueva.
- [ ] **Nombre del comando.** `restore` ya existe para snapshots. ¿`clawg memories undelete`, `clawg undelete`, otro? ¿Se acepta que `memories` deje de ser de solo lectura?
- [ ] **FTS: reinserción manual o trigger nuevo.** Manual en la transacción: sin migración, un camino. Trigger de la transición borrada→viva (V11): cubre cualquier camino futuro, pero suma una migración con su test de preservación y toca el invariante "una suma, una resta" de V8.
- [ ] **Tablas cubiertas.** ¿Sólo `memory` (lo único que se crea en producción) y 400 para `episodic` y `procedural`, o las tres desde el principio (más texto de embedding y tests)?
- [ ] **Dedup.** Si ya hay una fila viva con el mismo hash: ¿409 nombrando la viva, restaurar igual, o fundir (sumar contadores a la viva y dejar la borrada)?
- [ ] **Contradicciones.** ¿Se re-detectan al restaurar (`detect_contradiction` con el vector nuevo) o se restaura sin disputar?
- [ ] **Filas con secreto.** Si `redact` encuentra un secreto en el texto: ¿se rechaza y se indica `clawg purge`, se restaura y se avisa los tipos, o se pide una marca explícita desde la CLI?
- [ ] **Filas `superseded`.** ¿Se restauran y se avisa que siguen reemplazadas, o se rechazan?
- [ ] **Recencia y tier.** ¿Se conservan `last_accessed_at` y tier, o el restore cuenta como acceso (`last_accessed_at = ahora`)? Sin eso, una fila restaurada hace meses puede quedar viva e inalcanzable.
- [ ] **Descubrimiento por MCP.** ¿`list_memories` aprende `deleted`, o el agente se limita a ids que ya tiene? Mientras se mantenga el test de los cinco argumentos, la respuesta por defecto es no.
- [ ] **Scope.** Con scope (MCP, `?agent=`), ¿sólo las propias, como `delete_memory`? agent-sharing ya decidió quién modifica filas compartidas: sólo su autor; para los demás son de sólo lectura ([`specs/sharing`](../../specs/sharing/spec.md), R5).
- [ ] **Papelera con retención.** Hoy una borrada se queda para siempre salvo `purge`. ¿Se queda así (recomendado: la purga ya existe y borrar datos solos viola la regla de no destruir la base del usuario), o se agrega un vaciado automático?
- [ ] **Borrado por proyecto.** ¿Cambio aparte, después de éste, sólo CLI?

## Cómo se mide

Tests RED primero:

1. Restaurar una fila borrada: está viva, su vector existe, la búsqueda léxica (FTS) por una palabra suya la encuentra y la búsqueda KNN también.
2. Exactamente una entrada FTS: borrar, restaurar y volver a borrar deja el índice igual que un solo borrado (no hay doble resta).
3. Restaurar una fila de otro agente con scope responde 404 y no cambia nada: ni la fila, ni `last_modified_at`, ni `update_count`, ni el vector (patrón de `an_owner_mismatch_deletes_nothing`).
4. Restaurar una fila viva, una inexistente y una purgada dan el mismo `NotFound`, sin escribir nada.
5. Dedup: borrar, guardar el mismo texto, restaurar. No hay dos filas vivas iguales y el error nombra la viva.
6. Un vector huérfano preexistente no hace fallar la restauración.
7. Embedder no listo: 503 y la fila sigue borrada.
8. Disputas: una fila `disputed` contra una fila ya borrada vuelve en reposo; las que se liberaron al borrar siguen liberadas; si se decide re-detectar, un contradictor guardado mientras estaba borrada dispara la disputa.
9. Síntesis: restaurar una conclusión vuelve a cubrir a sus fuentes y restaurar una fuente la deja pendiente o cubierta según haya una conclusión viva. No se toca `memory_derivation`.
10. Secretos (según la decisión): una fila con un secreto de formato conocido no vuelve a la búsqueda sin el aviso o el rechazo decidido.
11. Cambio de modelo: una fila borrada antes de un `reembed_all` se restaura con el vector del embedder vigente, no con uno viejo.
12. Listado: `deleted=true` devuelve sólo borradas, por `deleted_at` descendente; el listado normal no cambia.
13. Base sembrada en la versión anterior (filas borradas por el delete de hoy, con el vector ya podado): se restauran y siguen alcanzables. Es la regla de `config.yaml` para datos guardados, y vale aunque no cambie el esquema.
14. Si hay MCP: los tests de nueve herramientas pasan a diez de forma deliberada; el scope por agente lo cubre un test como `an_agent_cannot_delete_another_agents_memory`.

`rag-eval --diff`: este cambio no toca ranking ni retrieval y el golden set no tiene filas borradas, por lo que no es requisito. Si se decide que el restore actualiza `last_accessed_at`, correrlo una vez de todos modos.

## Riesgos

- **Fila duplicada o invisible:** dedup y `superseded` (conflictos 1 y 5) son las dos formas de que "restaurar" parezca funcionar y no sirva. Cada una tiene su test.
- **Doble resta en el FTS:** corrompe el índice de forma silenciosa (`V8__memory_fts.sql:50-58`). Es el riesgo técnico mayor; por eso el test 2.
- **Secretos viejos de vuelta a la búsqueda:** ver conflicto 6 y su decisión.
- **Costo de contexto de la décima herramienta:** sin medir en el repo. Se puede elegir C/D para no pagarlo.
- **Id purgado reutilizable:** contradicción entre `AUTOINCREMENT` y `specs/synthesis` R6 (conflicto 9). Verificar antes de escribir la spec.
- **Estado del repo:** el `CLAUDE.md` del proyecto no lista `clawg memories`. Cualquier comando nuevo obliga a corregirlo.
