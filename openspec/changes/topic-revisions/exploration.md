# topic-revisions — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | M (append-only por clave) a L (reemplazo en el lugar con historial) | conflict-candidates (blanda; ver «Dependencia») | exploración cerrada; decisiones en la [propuesta](proposal.md#decisiones) |

**Tesis:** una decisión que evoluciona («la base es X», después «migramos a Y») hoy termina en **dos filas vivas**, y la vieja sigue puntuando en el ranking. El detector sólo la frena si las dos caen en su banda, y en ese caso lo único que hace es pedirle una decisión al usuario. No hay una forma de decir «esto reemplaza a aquello» sin conocer el id del viejo y sin perder su historia. La pregunta de diseño es si la revisión **reemplaza el contenido de una fila** (conserva id, accesos y tier, pero rompe la inmutabilidad que sostiene varios índices) o **encadena una fila nueva** (más filas, pero todo lo que existe sigue siendo cierto).

## Qué de la premisa es cierto y qué no

| Afirmación | Hoy |
|---|---|
| El contenido es inmutable | **Cierto, y está escrito como invariante.** «There is no trigger on `content`. It is write-once: no statement anywhere updates it» (`crates/clawg-core/migrations/V8__memory_fts.sql:90-93`); `specs/update`: «El contenido no se cambia: para eso se guarda el texto nuevo y se borra el viejo». Un `rg` de `SET content` en `src` sin tests no encuentra ningún `UPDATE` de contenido; los únicos que tocan la fila son `content_hash` (`dedup.rs:176`, `:214`). |
| No hay upsert; la dedup es por hash | **Cierto.** Sólo un texto **idéntico** (mismo agente, proyecto y `content_hash`) refuerza una fila viva en lugar de insertar (`store_insert.rs:236-243`, `worker.rs:483`). Cualquier reescritura es otra fila. |
| `update_memory` sólo cambia importancia y tipo | **Cierto** (`specs/update` R1; `MemoryPatch`, `admin.rs:159`). |
| «Termina como dos filas y, en el mejor caso, el detector las disputa y pregunta al usuario» | **Parcial.** Hay tres caminos: (1) el texto nuevo cae en la banda `[0,10, 0,42)` y es claim-bearing: se disputa (`contradiction.rs:58`, `:66`); (2) cae fuera de la banda (una decisión reescrita más larga, por ejemplo): **no hay ninguna marca**, las dos puntúan igual y la vieja sigue apareciendo; (3) el agente conoce el id y hace `save_memory` + `delete_memory`, que hoy no se deshace y pierde id, accesos y tier. El caso (2) es una inferencia desde las distancias del detector, no una medición: se mide en la compuerta (ver «Cómo se mide»). |
| «La historia ya existe de algún modo» | **Parcial y relevante.** Una fila `superseded` conserva contenido, vector y FTS, sigue en `list_memories` y se recupera resolviendo al revés (`specs/disputes` R1, R3; `dispute.rs:189-193`). Es un historial encadenado (`superseded_by`), pero sólo se llega a él por el detector. |
| El guardado es asíncrono y responde 202 | **Parcial.** Por defecto el handler espera el informe del worker hasta `SAVE_CONFIRM_TIMEOUT` (10 s, `daemon/api.rs:90`) y responde **201 `saved`** con los ids; sólo responde **202** si se acaba el plazo (`unconfirmed`) o el embedder todavía carga (`embedder_loading`) (`api.rs:640`, `:746`). Importa porque la revisión cambia lo que el informe puede decir (abajo). |

## Problema (verificado contra el código)

### Todo lo que hoy asume que el contenido no cambia

| Pieza | Dónde | Qué pasaría con un reemplazo en el lugar |
|---|---|---|
| Índice FTS | Externo y **sustractivo**: cada fila viva está sumada una vez; la resta toma el contenido exacto (`V8__memory_fts.sql:50-62`). Los triggers cubren inserción, borrado lógico y físico, no cambio de `content` (`:90-93`) | Hay que restar los términos viejos y sumar los nuevos en la misma transacción, o agregar un trigger nuevo (una migración con su prueba de preservación). Un error corrompe el índice en silencio |
| Vector | `memory_vectors(rowid, embedding)` (`V2__vectors.sql`) | Reemplazar el vector del mismo `rowid`; el vec0 ya obliga a borrar y reinsertar en `reembed_all` |
| `content_hash` e índice de dedup | `idx_memory_dedup` parcial sobre vivas (`V6__content_hash.sql:14`) | Recalcular el hash; sin eso, volver a guardar el texto nuevo no reforzaría la fila y volver a guardar el viejo la reforzaría con el contenido equivocado |
| Piezas de un save largo | `memory_piece`, `save_id` es un contador aparte y `memory_id` **no es único** (`V11__memory_piece.sql`, `pieces.rs:187-194`) | Un texto nuevo puede tener otra cantidad de piezas; y una pieza absorbida por la dedup queda **enlazada también a un segundo save** (`worker.rs:575-579`), así que reemplazar su contenido cambia otro save sin avisar |
| Re-embed | Reconstruye el vector de cada fila viva con su `context` de cabecera (`maintenance.rs:640-654`) | Debe seguir viendo el contenido y la cabecera del texto vigente |
| Conclusiones de síntesis | `memory_derivation` apunta a las fuentes; una fuente cubierta sólo exige estar viva y no `superseded` (`synthesis.rs:32-38`), no que su texto sea el original | Una conclusión sacada del texto viejo seguiría «cubriendo» a una fuente cuyo texto cambió. En un encadenado, la fuente vieja queda `superseded`: la conclusión deja de cubrirla y la fuente nueva aparece pendiente, que es lo correcto |
| Pares de disputa | `disputed_against` y, si existe, la tabla de candidatas (conflict-candidates) | Los pares de una fila cuyo texto cambió ya no describen nada |
| Purga y secretos | `purge` borra `memory`, el vector, `memory_derivation` y el FTS (`secrets.rs:93-100`); `clawg secrets` escanea sólo `memory` (`secrets.rs:41`) | Una tabla de versiones guardaría el texto viejo fuera de ambos |

### Dos hallazgos que el cambio no puede ignorar

- **`purge` no borra `memory_piece`** (`secrets.rs:93-100` no la nombra; el `rg` de `memory_piece` en `crates` sólo la encuentra en `pieces`, `search`, `worker`, `maintenance` y migraciones). Queda un vínculo con el `context` (el comienzo del save, ya enmascarado). No es un secreto, pero un diseño nuevo no debe copiar el hueco.
- **Filas anteriores al enmascarado.** El enmascarado ocurre en el worker, antes de partir, hashear y embeber (`worker.rs:430`). Las filas de antes pueden tener un secreto en texto plano. Una tabla de versiones que copie ese texto lo duplica; una revisión que lo reemplaza lo saca de la fila viva pero lo deja en el historial.

## Lo que ya existe y se reutiliza

- El worker es el **único consumidor secuencial** de la cola (`worker.rs:206-211`): una revisión puede resolverse dentro de la etapa 3 sin carreras, y dos revisiones de la misma clave en cola se aplican en orden.
- El embedding se hace **antes** de tocar la base (`worker.rs:508-518`, etapa 2) y una falla de embedding devuelve el informe sin escribir nada. La revisión hereda esa atomicidad: el texto nuevo se embebe donde ya se embebe un save, y la transacción de la etapa 3 solo cambia filas.
- Enmascarado, troceo y hash ya corren antes de cualquier decisión (`worker.rs:430-473`).
- `MemoryEvent` ya sabe agregar campos opcionales sin romper lo anterior: `#[serde(default)]` en `memory_type`, `importance` y `derived_from` (`events.rs:30-43`). Un evento en el archivo de spill de una versión anterior se lee igual (`spill.rs`).
- `siblings` en `detect_contradiction` es una lista de filas que se dejan fuera (`contradiction.rs:143`, `worker.rs:541`): sirve para excluir la fila que la revisión reemplaza.
- `resolve_dispute_core` y `SUPERSEDE_SQL` (`dispute.rs:192`, `:198`): el encadenado sin mutar contenido ya existe y está probado, con su vuelta atrás.
- `retier` (`tier.rs:115`, `specs/update` R2): regla pura de qué tier queda cuando cambian importancia o tipo.
- `redact(&str)` es pura (`redact.rs:131`): permite enmascarar el texto de una versión al copiarlo.
- Regla de acceso: un agente sólo modifica filas propias; lo compartido es de lectura (`specs/sharing` R5, `specs/visibility` R3-R4).

## Enfoques posibles

La clave es un argumento opcional de `save_memory` (`topic_key`), con la misma forma que `derived_from` (`SaveMemoryArgs`, `server.rs:123-145`; `SaveArgs`, `client.rs:288-298`). Alcance de la clave: **agente + proyecto + clave**, con `COALESCE(project_id, '')` igual que la dedup (`V6__content_hash.sql:14`). Cambian el efecto de guardar con una clave que ya tiene fila viva.

### E1. Reemplazo en el lugar con tabla de versiones (el del pedido)

Si hay una fila viva con la misma clave: se copia el contenido anterior a `memory_version`, se reemplaza el contenido, se re-embebe, se recalcula el hash, se suma `update_count` (`V1__init.sql:19`) y se conservan **id, accesos y tier**. Contenido idéntico: no hace nada, sin versión vacía (ya lo hace la dedup por hash, que refuerza).

- A favor: el id es estable; lo que cita el id (conclusiones `derived_from`, pares de disputa, el workspace, lo que el agente anotó) sigue apuntando a lo vigente. El historial no engorda `list_memories` ni la búsqueda.
- En contra: todas las filas de la tabla de arriba se vuelven trabajo. El FTS es el más peligroso. La tabla de versiones es un lugar nuevo con texto: `purge`, `clawg secrets` y los snapshots (`specs/secrets` R6) tienen que cubrirla. Una conclusión sacada del texto viejo no se entera.
- Una pieza compartida con otro save (arriba) obliga a **no** reemplazarla en el lugar; esa pieza se desengancha (fila nueva) y queda la vieja para el otro save. Es una rama más.

### E2. Encadenado por clave, sin mutar contenido (recomendado como primer paso)

Si hay una fila viva con la misma clave: se inserta una fila **nueva** (con la clave y el texto nuevo, por el camino de siempre) y, en la misma transacción, la vieja pasa a `superseded` con `superseded_by` apuntando a la nueva. La nueva hereda tier y `access_count` de la vieja. La historia es la cadena de filas `superseded`, ya listable y reversible con `resolve_conflict` al revés.

- A favor: **ninguna invariante cambia.** FTS, vector, hash, piezas, re-embed, `purge` y `clawg secrets` ya funcionan sobre filas completas. El historial es lectura de lo que existe (`list_memories`, `read_memory`). La conclusión de síntesis queda correcta sola (la fuente vieja deja de cubrir). La tabla nueva no existe; sólo una columna `topic_key` y un índice parcial.
- En contra: **el id cambia** en cada revisión; lo que cita el id viejo debe seguir la cadena. Las filas viejas ocupan espacio (vector y FTS) en lugar de un texto suelto. `list_memories` muestra las `superseded` entre las demás (hoy ya pasa con las resueltas).
- La pieza compartida entre saves (arriba) también existe acá: reemplazar un save largo marcaría `superseded` filas que otro save sigue usando. Se resuelve igual: sólo se reemplazan las filas que pertenecen **únicamente** al save reemplazado.
- El detector tiene que dejar fuera la fila que va a reemplazar (parámetro `siblings`), o el par nace y muere en la misma transacción.

### E3. `replaces: <id>` en el save, sin clave

El agente pasa el id del que reemplaza; mismo efecto que E2. Acepta una **lista de ids** (`replaces: [id, ...]`) y es la **única forma de declarar el reemplazo al guardar**: no se agrega un segundo mecanismo en otro cambio. Reutiliza la semántica del veredicto `supersedes` y el camino de escritura que define conflict-candidates (24): `resolve_dispute_core` (`dispute.rs:198`), que deja la vieja `superseded` con `superseded_by` (columna que ya existe, `V1__init.sql:24`). La entrada natural son las candidatas que 24 devuelve en la respuesta del save (enfoque E de 24): el agente las ve, decide que la nueva reemplaza a alguna y las pasa en su siguiente save o las cierra con `resolve_conflict`.

- A favor: sin columna, sin índice, sin validación de formato.
- En contra: obliga a buscar el id antes de guardar (una llamada más, y la búsqueda puede traer otra fila); no sirve cuando la decisión se vuelve a revisar en otra sesión sin el id a mano. La clave es lo que da estabilidad entre sesiones.

### E3b. `content` en `update_memory`

Es la misma semántica que E3 (una fila nueva y la vieja `superseded`), pero por la herramienta que ya recibe el id: `update_memory(kind, id, content)`.

- A favor: no suma un argumento a `save_memory`, y el agente ya llega con el id, porque se lo devolvió el save o la búsqueda.
- En contra: hoy `update_memory` conserva el id y sólo cambia importancia y tipo (`specs/update` R1; el `UPDATE` sólo toca `importance`, `memory_type`, `tier`, `last_modified_at` y `update_count`, `crates/clawg-core/src/memory/admin.rs:195-197`). Con E2, el id **cambia**, y una herramienta que se llama «update» y devuelve otro id confunde. Si se elige, la respuesta tiene que nombrar el id nuevo y el viejo.
- Se decide junto con E3: son dos puertas a lo mismo, y conviene una sola. E3b gana si la medición de 24 muestra que los reemplazos se deciden después del save, con el id en la mano. E3 gana si se deciden al guardar.

### Comparación

| | E1 reemplazo + versiones | E2 encadenado por clave | E3 `replaces` |
|---|---|---|---|
| Id estable | sí | no | no |
| Invariantes que cambian | FTS, hash, vector, piezas, purge, secrets | ninguna | ninguna |
| Migración | columna + tabla + (trigger FTS) | columna + índice | ninguna |
| Historial visible por | herramienta nueva o CLI | `list_memories` / `read_memory` (ya existen) | igual que E2 |
| Costo de espacio | bajo (sólo texto) | alto (vector y FTS por versión) | igual que E2 |
| Tamaño | L | M | S |

**Recomendación:** E2 como primer paso. Pasar a E1 sólo si aparece un uso real del id estable (referencias a lo vigente que no puedan seguir una cadena); los costos de E1 son altos y los de E2 casi nulos. Esta recomendación es una lectura de la tabla de arriba, no una medición. Que E3 acepte la lista de ids que devuelve 24 **no cambia** la recomendación: E3 resuelve el reemplazo puntual con ids a mano; la clave de E2 sigue siendo lo que da estabilidad entre sesiones sin tener que buscar el id. Si la medición de 24 muestra que casi todo reemplazo se resuelve en el momento con los ids de la respuesta del save, E3 gana peso y se puede entregar primero (es S y no migra); se revisa entonces. Las decisiones de la clave (formato, alcance, lookups) valen para las tres.

## Descartado

- **Contenido mutable sin historial.** Pierde el texto anterior, que puede ser el correcto; contradice la regla de no destruir lo que el usuario guardó (`openspec/config.yaml`, `rules.general`); y no tiene vuelta atrás. Un historial de una sola versión (el inmediato anterior) tampoco: una segunda revisión lo pisa.
- **API de buscar y reemplazar** (`edit_memory(old, new)`). Exige que el agente cite el texto exacto (hay que leer la memoria antes, y para un save largo, todas las piezas con su solape: `read_memory` ya existe pero devuelve el texto armado, `pieces.rs`); cambia el contenido en el lugar, con todos los problemas de E1; y suma una herramienta cuando los tests fijan el conteo (`server.rs:1361-1384`). El reemplazo parcial no da nada que el reemplazo total con clave no dé.
- **Sugerir una clave por heurística de texto** (sin LLM). Una clave mal sugerida se cuela en la siguiente revisión y reemplaza una memoria **no relacionada** sin que nadie la elija. Un reemplazo equivocado es peor que ningún reemplazo. Alternativa más segura, sin heurística: si un save sin clave cae en la banda del detector contra una fila viva del mismo tipo, la respuesta del save puede decir «parece una revisión de #N; pasá `topic_key` para reemplazarla». Usa algo que ya existe, y no es la única vía: el aviso en la respuesta sólo lo ve el agente que guardó (la misma advertencia de conflict-candidates).
- **Un intervalo de validez (`valid_from`/`valid_to`).** No hace falta mientras la cadena de filas `superseded` (`superseded_by`) más `created_at` den la historia. Hoy ningún lado lo escribe ni existe la columna (no aparece `valid_from` ni `valid_to` en `crates` ni en `migrations`). Se reabre sólo si aparece un uso que pida «qué era cierto en la fecha X».
- **Una `topic_key` por proyecto sola, sin agente.** Dejaría a un agente reemplazar la memoria de otro. La clave se resuelve dentro del agente.

## Interacciones

- **Guardado asíncrono.** El save con clave entra a la cola como cualquier otro; la decisión «reemplazo / fila nueva» se toma en la etapa 3 del worker, donde ya se hace el insert. La etapa 1 (compuerta de hash) corre antes: si el texto es **idéntico** al de la fila vigente, la dedup lo refuerza (no-op, sin versión nueva) y es lo pedido. Si es idéntico al de **otra** fila, también se refuerza **esa**, y la fila con la clave no se actualiza: borde a decidir. El informe del save gana un tercer resultado además de `inserted` y `reinforced` (`api.rs:455-466`; el cliente documenta sólo esos dos, `client.rs:305`); un puente viejo debe tolerar el valor nuevo.
- **Re-embed.** Ocurre en la etapa 2, con el embedder del worker, igual que un save (`worker.rs:508`). No hay un camino de re-embed nuevo; el vector nuevo entra por `insert_memory_with_vector` en E2 o reemplaza al viejo en E1.
- **Spill y replay.** `topic_key` va con `#[serde(default)]` (`events.rs:30-43`): un spill de una versión anterior se repone como save normal, y uno de la nueva versión repuesto por una anterior pierde la clave (la guarda como fila nueva, sin reemplazar). No pierde el texto.
- **Argumento compartido 24 → 27.** El orden 24 → 27 también lleva este argumento: 24 devuelve en la respuesta del save los ids de las candidatas y define el veredicto `supersedes`; 27 (E3) acepta esos ids en `replaces` y escribe por el mismo `resolve_dispute_core`. Un solo mecanismo de reemplazo, no dos.
- **conflict-candidates.** Una revisión cambia o reemplaza el texto de una fila, y los pares pendientes de la fila reemplazada pierden sentido. Con la tabla de candidatas hay dónde cerrarlos; con sólo `disputed_against` hay que reescribirlo. Y la clave sirve de segunda señal para excluir un par (nunca se compara una fila con otra de su misma clave).
- **recoverable-delete.** La búsqueda de la clave exige fila viva. Si se borra la fila de una clave y se guarda de nuevo, nace otra con la misma clave; restaurar la vieja produce **dos vivas con la misma clave**, el mismo problema que la dedup (conflicto 1 de recoverable-delete). Restaurar tiene que rechazarlo nombrando la viva, o la clave debe ser única entre vivas (índice parcial).
- **Sharing.** La clave se resuelve dentro del agente (`agent_id` en la clave de búsqueda). Un agente no revisa filas ajenas aunque las vea en una búsqueda (`specs/sharing` R5), y dos agentes pueden usar la misma clave sin tocarse.
- **Piezas.** Ver la tabla de arriba: reemplazar un save largo toca todas sus piezas, y sólo las propias de ese save.
- **Secretos.** El texto nuevo se enmascara antes de todo (`worker.rs:430`). En E1 el texto copiado a la tabla de versiones se pasa por `redact` y la tabla entra en `purge` (por `memory_id`) y en `clawg secrets`. En E2 no hay lugar nuevo.
- **Síntesis.** Ver la tabla: E2 no cambia nada; E1 puede dejar una conclusión cubriendo un texto que ya no existe.
- **conclusion-upkeep (45).** Con E2, una fuente revisada queda `superseded`, que es justo el estado que 45 usa para mandar una conclusión a revisión. Además, 45 propone que `derived_from` rechace una fuente `superseded`, para que una conclusión nueva no se apoye en la versión reemplazada.
- **history-import (44).** En la fuente medida por 44, el 75 % de las notas trae una clave de tema. Si se importa antes que exista la columna, la clave se pierde, y la próxima revisión de esa decisión entra como una fila nueva y no como revisión. Por eso 27 va antes que 44.
- **memory-provenance (41).** Una revisión lleva su propio origen, el de quien la escribió, no el de la fila que reemplaza.

## Dependencia

La relación 24 → 27 es **blanda**. Ninguno de los dos necesita al otro para existir, pero conviene este orden:

- La revisión invalida los pares pendientes de la fila reemplazada. Con una tabla de candidatas (conflict-candidates) eso es cerrar filas de una relación; sin ella, reescribir un JSON dentro de `memory` o dejar `disputed_against` apuntando a una fila `superseded`, que `rest_status` ya ignora (`dispute.rs:108-114`) pero que deja el par «fantasma» en `disputed_against`.
- Sin conflict-candidates, la exclusión «nunca comparar una fila con su propia clave» alcanza para no generar pares nuevos y se puede entregar; el resto es limpieza.
- Hacia el otro lado, conflict-candidates puede usar la clave como segunda señal, pero es una extensión posterior, no un requisito.

Por eso `depends_on: [conflict-candidates]` queda como se planeó, con ese motivo. Si el usuario prefiere entregar topic-revisions antes, es viable con E2 y la exclusión por clave; lo que se pierde es la limpieza de pares.

## Alcance candidato

1. Columna `topic_key` (nullable) en `memory` e índice parcial `(agent_id, COALESCE(project_id,''), topic_key)` sobre vivas. Migración nueva; el checksum entra en los fijados (`migrations.rs:43-55`).
2. Función pura de validación de la clave (formato, largo), con su tabla de casos.
3. `topic_key` opcional en `save_memory`, `POST /v1/memories` y `MemoryEvent`, con `#[serde(default)]`.
4. Camino de revisión en el worker, según el enfoque elegido, con resultado propio en el informe.
5. Exclusión de la fila reemplazada en el detector.
6. Un save largo con clave reemplaza todas las piezas **propias** de su save anterior.
7. `read_memory` (o el CLI) muestra la clave y la cadena; decisión abierta si el agente la lee.
8. Docs: `CLAUDE.md` del repo y las specs `update`, `pieces`, `secrets`, `disputes` y `mcp`.

### No-alcance

- Sugerir claves por heurística.
- Edición parcial de texto.
- Revisar filas de otro agente.
- Búsqueda por clave dentro de `search_memory` (cambia retrieval; se evalúa aparte, ver decisiones).
- Una herramienta MCP nueva para ver el historial.
- Cambios de ranking: ninguno.
- Deshacer una revisión como operación propia en E2: ya se deshace resolviendo el par al revés; en E1 sería una operación nueva.

## Decisiones abiertas

Son del usuario. Quedaron tomadas en la [propuesta](proposal.md#decisiones): E2, sin puerta por id, clave con el formato propuesto y alcance agente + proyecto, herencia de tier, accesos, tipo e importancia, y revisión de preferencias y decisiones sin pasar por el usuario.

Desde que se escribió esta exploración se archivó conflict-candidates: la banda ya no marca `disputed` toda candidata (sólo los pares escalados), las candidatas vuelven en `related`, y el spill se reemplazó por la outbox (`ingest_outbox`). Las citas `path:line` de arriba son de antes de ese cambio; la propuesta las vuelve a citar contra `main`.

- [ ] **Enfoque.** E1 (reemplazo en el lugar con historial, el del pedido), E2 (encadenado por clave, recomendado como primer paso) o E3 (`replaces: <id>`). Decide también si se acepta que el id cambie.
- [ ] **Puerta del reemplazo por id.** ¿`replaces` en `save_memory` (E3) o `content` en `update_memory` (E3b)? Una sola de las dos.
- [ ] **Formato de la clave.** Propuesta: segmentos en minúscula `[a-z0-9._-]` separados por `/`, máximo 80 caracteres, rechazo con mensaje que nombra el campo. ¿Se acepta, o la clave es texto libre?
- [ ] **Alcance de la clave.** Propuesta: agente + proyecto + clave. Una `user_preference` visible en todos los proyectos queda con una fila por proyecto si se guarda con la misma clave desde carpetas distintas. ¿Se acepta o las preferencias llevan clave global?
- [ ] **Pieza compartida con otro save.** ¿Se desengancha (fila nueva) o se rechaza la revisión con un error que la nombra?
- [ ] **Identidad textual con otra fila.** Si el texto nuevo es idéntico al de **otra** fila, la dedup la refuerza y no actualiza la clave. ¿Se deja así, o la compuerta prefiere la fila con la clave?
- [ ] **Qué hereda la revisión (E2).** ¿Tier y `access_count` de la anterior (propuesta), sólo el tier, o nada? ¿Y `importance` y `memory_type`: los que traiga el save, o los de la anterior si el save no los trae?
- [ ] **Quién lee las versiones.** Con E1: ¿`read_memory` las muestra, o sólo `clawg memories` (visor del humano, fuera del alcance del modelo)? Con E2: la cadena se ve con `list_memories`; ¿alcanza?
- [ ] **Retención de versiones.** Con E1: ¿todas, o las últimas N? Borrar versiones destruye texto del usuario; la regla de `config.yaml` empuja a conservar todas y a que sólo `purge` borre.
- [ ] **Secreto viejo al copiar (E1).** ¿Se enmascara con `redact` al copiar a la tabla, o se copia tal cual y `clawg secrets` la lista?
- [ ] **Aviso de posible revisión.** ¿La respuesta de un save sin clave que cae en la banda sugiere «pasá `topic_key`»?
- [ ] **Búsqueda por clave.** ¿`search_memory` resuelve una consulta con forma de clave por coincidencia exacta antes del KNN? Cambia retrieval y exige `rag-eval --diff` vacío para consultas sin clave.
- [ ] **Unicidad entre vivas.** ¿Índice único parcial o sólo comprobación en el worker (la dedup hace lo segundo, `V6__content_hash.sql:9-12`)?

## Cómo se mide

### Compuerta previa

Antes de escribir código, medir el problema con el modelo real y el mismo worker del daemon (`EvalIngest`, `eval_api.rs:457`): un set de ≥ 30 decisiones que evolucionan en 2 o 3 versiones, de tamaño corto y largo, ES y EN. Para cada versión, guardar y registrar: cuántas veces la fila vieja queda `disputed` (dentro de la banda), cuántas queda `active` **sin** marca (fuera de la banda) y qué posición ocupan la vieja y la nueva en `search_memory` con la consulta de la última versión. Es lo que dice si el caso (2) de la tabla de premisas es frecuente o raro; si es raro, la prioridad baja.

### Tests RED primero

1. Guardar con una clave que no existe inserta una fila con la clave. Guardar con la clave de una fila viva produce la revisión según el enfoque elegido: la búsqueda por el texto nuevo la encuentra y por el viejo **no** (FTS y KNN), y sólo hay una fila viva con la clave.
2. Texto idéntico con la misma clave: no-op. Ninguna versión ni fila nueva; el resultado es `reinforced`.
3. Guardar de nuevo el texto **anterior** después de una revisión es otra revisión, no un refuerzo de la fila equivocada (el hash vigente es el del texto actual).
4. FTS (E1): el índice queda igual que con una sola inserción del texto nuevo, sin términos viejos y sin doble resta (el invariante de `V8__memory_fts.sql:50-62`).
5. Save largo con clave: se reemplazan todas las piezas **propias**; una pieza compartida con otro save no cambia ese otro save; la cantidad de piezas puede ser otra.
6. El detector no compara la fila con su propia clave (ni la reemplazada) y sigue disputando a las demás.
7. Pares pendientes de la fila reemplazada quedan cerrados o recalculados (con conflict-candidates) y ninguno apunta a texto que ya no existe.
8. Otro agente con la misma clave no toca la fila (alcance por agente) y no ve la versión como suya; con otro proyecto, otra fila.
9. Cola: dos revisiones de la misma clave en cola se aplican en orden y la fila final tiene el segundo texto; una falla de embedding no escribe nada y el informe lo dice (`worker.rs:519-531` hace lo mismo con vectores faltantes).
10. Spill: un evento sin `topic_key` (formato anterior) se repone como save normal; uno con clave se repone como revisión.
11. Secretos: el texto nuevo sale enmascarado; `clawg purge` y `clawg secrets` cubren todo lugar nuevo con texto; con E1, una fila anterior al enmascarado revisada no deja el secreto en la tabla de versiones (según la decisión).
12. Rechazo de claves inválidas con un mensaje que nombra `topic_key`.
13. Formato del resultado: un puente anterior que no conoce el resultado nuevo sigue mostrando el save.
14. **Base sembrada en la versión anterior** (regla de `config.yaml`): una base V12 con filas, piezas y pares abre con la migración, las filas quedan con clave `NULL`, siguen alcanzables por búsqueda y un save sin clave se comporta como hoy. Un binario anterior abre la base migrada (la columna extra no lo rompe) y trata una fila `superseded` encadenada como cualquier otra.
15. recoverable-delete: restaurar una fila cuya clave ya tiene otra viva se rechaza nombrándola (o lo que se decida).

### `rag-eval --diff`

Ningún caso del golden set lleva clave, así que el ancla debe dar diff vacío: ranking y recuperación no cambian para filas sin clave. Si se decide la búsqueda por clave en `search_memory`, `--diff` vacío para todas las consultas sin forma de clave y un caso nuevo para las que sí la tienen. Si se elige E2, correr el agent-view una vez con un set que incluya cadenas, para ver que las `superseded` no suben tokens de `recall_context`.

## Riesgos

- **Reemplazar una memoria equivocada.** La clave es una decisión del agente, y una colisión de claves reemplaza algo no relacionado. El historial lo salva en E1 y E2, y las claves heurísticas se descartan por eso. Falta decidir si la respuesta del save dice qué reemplazó (y con qué id) para que se vea.
- **Corrupción silenciosa del FTS (E1).** El trabajo más delicado; un test por camino.
- **Pieza enlazada a dos saves.** Cambiar su contenido (E1) o reemplazarla (E2) afecta a otro save. Hay que fijar el comportamiento y la decisión.
- **Conclusiones que quedan atrás (E1).** Ver la tabla de síntesis.
- **Una tabla con texto que escapa a `purge` (E1).** La regla es que todo lugar con texto del usuario entre en `purge`, `clawg secrets` y los snapshots; hoy `memory_piece` ya es un hueco menor (arriba).
- **Dos vivas con la misma clave** tras un restore o una condición de carrera (el worker es secuencial, restaurar no lo es). Un índice único parcial lo prohíbe, a costa de que el restore falle en vez de duplicar.
- **Costo en contexto del campo.** Un argumento más en `save_memory` suma tokens a todas las sesiones; el repo no lo mide (context-density trabajó sobre `recall_context`, no sobre las herramientas). Se mide con el tokenizador real antes de cerrar.
- **Cambio de contrato del save.** El informe gana un resultado, y puentes y daemons de versiones distintas conviven (`version_warning`, `crates/clawg-cli/src/memories.rs`).
- **Estado del repo:** el `CLAUDE.md` del proyecto debe describir el argumento nuevo y la regla de reemplazo.
