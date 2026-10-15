# ingest-outbox — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | M | — (conviene que cierre antes de 25; se coordina con 18, 31 y 32; ver «Interacciones») | exploración (borrador) |

**Tesis:** un save que el daemon aceptó y todavía no insertó vive sólo en RAM. Esa ventana es justo el arranque de cada daemon (el modelo está cargando y `clawg mcp` lo autoarranca al empezar la sesión) y cualquier save cuya confirmación vence. Un kill, un crash o una señal de terminar en esa ventana pierde saves que el agente ya recibió como `202`, y un fallo después de aceptar sólo llega al log: el agente nunca se entera. Hoy «aceptado» no promete nada. Debería querer decir «se guardará, o se reportará como fallido». El cambio es mover la verdad de la cola en RAM a una tabla de `memory.db` (un outbox), escrita ya enmascarada antes de contestar y borrada en la misma transacción que inserta la memoria.

## Premisas que se verificaron y cuáles no eran del todo ciertas

| Premisa | Resultado |
|---|---|
| La cola es un `mpsc` acotado en RAM, con un único worker secuencial | **Cierta.** `INGEST_QUEUE_CAPACITY = 1024` (`crates/clawg-core/src/runtime.rs:45`), canal creado en `runtime.rs:134`, consumido por `memory_worker` (`crates/clawg-core/src/memory/worker.rs:198-275`). El único sitio de producción que lo lanza es `crates/clawg-cli/src/daemon/serve.rs:102`. |
| Con el embedder `ready` el handler espera hasta 10 s y contesta `201`/`500`/`503`; sólo contesta `202` si vence o si el embedder carga | **Cierta.** `SAVE_CONFIRM_TIMEOUT` (`crates/clawg-cli/src/daemon/api.rs:90`), `wait` según el estado (`api.rs:707-713`), espera con `timeout` (`api.rs:748-752`), mapeo en `save_response` (`api.rs:638-666`). `202 embedder_loading` sale sin canal de respuesta (`api.rs:745-747`). Cola llena: `503 ingest_queue_full` (`api.rs:739`). Embedder `failed`: se rechaza antes de encolar (`api.rs:707-710`). |
| Un apagado ordenado vuelca lo no ingerido a `ingest-pending.jsonl`, que se reinyecta al arrancar | **Cierta.** `shutdown_by` (`runtime.rs:219-246`) espera `DRAIN_DEADLINE` de 10 s (`runtime.rs:52`) y escribe el resto con `spill::write` (`runtime.rs:229-241`); el worker lo carga al arrancar (`runtime.rs:363`, `spill.rs:62-94`) y lo borra al terminar (`spill.rs:47-57`); las líneas ilegibles van a `ingest-pending.rejected.jsonl` (`spill.rs:75-92`). |
| Sólo se maneja `ctrl_c`: no hay SIGTERM ni cierre de consola | **Cierta, y más fuerte en producción.** `rg` sobre `crates/` por `SignalKind`, `signal::`, `ctrl_close`, `ctrl_shutdown` y `SetConsoleCtrlHandler` encuentra únicamente `tokio::signal::ctrl_c()` (`serve.rs:167-190`). Además, en Windows el daemon autoarrancado nace con `DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP` (`crates/clawg-cli/src/client.rs:868-873`): no tiene consola, así que ni el Ctrl-C llega. La única vía ordenada de un daemon autoarrancado es `POST /v1/shutdown` (`clawg stop`). Todo lo demás (matar el proceso, cerrar sesión, apagar la máquina) pasa por encima del volcado. |
| Después de un `202` un fallo sólo va a `log::warn`, y el texto de MCP manda a mirar `list_memories` | **Cierta, con un matiz.** El warn está en `worker.rs:257-264` y la respuesta a un solicitante que ya se fue se descarta (`worker.rs:269-271`). Pero sólo `unconfirmed` manda a `list_memories` (`crates/clawg-cli/src/mcp/server.rs:466-468`); `embedder_loading` promete «it will be stored once the embedding model finishes loading» (`server.rs:463-465`), una promesa que hoy no se puede cumplir ante un kill. |
| `/health` no tiene contadores de ingesta | **Cierta.** `HealthBody` (`api.rs:409-426`) y `health` (`api.rs:610-626`) no leen la cola ni el store. |
| Un save largo se inserta pieza por pieza, cada una en su transacción; el vínculo `memory_piece` va aparte y su fallo sólo se avisa | **Cierta.** Una transacción por pieza en `insert_memory_with_vector` (`crates/clawg-core/src/memory/store_insert.rs:351-399`), bucle en `worker.rs:543-570`, `link_save` separado (`worker.rs:571`, `583-598`) con `log::warn!` si falla (`worker.rs:595-597`). Los tests fijan el prefijo parcial (`worker.rs:1889-1917`). El embedding es un único batch por evento (`worker.rs:508-510`). |
| `synchronous = NORMAL` en WAL: lo commiteado sobrevive a un crash del proceso | **Cierta, y limita lo que el cambio puede prometer.** `crates/clawg-core/src/memory/db.rs:195-205` (el comentario de `db.rs:186-189` dice «an fsync per checkpoint rather than per commit»). Es un hecho de SQLite: con `NORMAL` en WAL el commit no se sincroniza a disco, así que sobrevive a un kill del proceso pero los últimos commits pueden perderse en un corte de luz o un crash del sistema. **El outbox, tal como se plantea, cubre kill, crash y señal de terminar, no el corte de luz**, salvo que la inserción del outbox use `FULL` (decisión abierta). |
| El dedup absorbe una reinyección | **Cierta, con tres salvedades.** Un chunk repetido refuerza la fila existente y no inserta otra (`REINFORCE_LOOKUP_SQL`, `store_insert.rs:236-243`; `reinforce_if_duplicate`, `store_insert.rs:280-325`). Salvedades: (1) cada reinyección suma `access_count + 1` (`store_insert.rs:310-317`), así que un replay infla el conteo y puede subir el tier; (2) la consulta sólo mira filas `active`/`disputed` y no borradas (`store_insert.rs:241-242`): si la primera copia se borró o quedó `superseded` antes del replay, el replay inserta una fila nueva; (3) un save largo reinyectado a medias reabsorbe sus primeros pedazos y los vuelve a vincular sin la cabecera con la que se embebieron (ver «Problema»). |
| El enmascarado de secretos ocurre antes de trocear y el volcado guarda texto ya enmascarado | **Falsa para el volcado.** El enmascarado se hace en el worker (`redact` en `worker.rs:430`), no en el handler: el handler arma el evento con `body.content` tal cual (`api.rs:715-730`, en particular `api.rs:721`) y nunca llama a `redact`. El volcado serializa los eventos encolados sin pasar por `redact` (`runtime.rs:229-241`, `spill.rs:109-118`), así que `ingest-pending.jsonl` guarda el texto **crudo**. El diseño de secret-hygiene lo aceptó porque el volcado «existe sólo entre un apagado con eventos pendientes y el próximo arranque» (`openspec/changes/archive/2026-10-14-secret-hygiene/design.md:19-23`). Un outbox dentro de `memory.db` rompe esa premisa: la fila puede vivir mientras el save esté pendiente o fallido, y entra en cada snapshot. |
| Hay rutas en proceso que lanzan `ingest_worker` sin handle de drenaje | **Distinta: son fixtures de test.** `crates/clawg-cli/src/mcp/mod.rs:195` está en el `mod tests` de `mod.rs:142`; `crates/clawg-cli/src/client.rs:1025` en `pub(crate) mod tests` (`client.rs:1000-1001`); `crates/clawg-cli/src/mcp/server.rs:1030` en el `mod tests` de `server.rs:998`. También `crates/clawg-cli/tests/api.rs` y `crates/clawg-core/tests/public_api.rs`. Sirven para levantar un daemon en memoria (`MemoryRuntime::new_for_test`, `runtime.rs:281-323`). Aparte, `EvalIngest` lanza `memory_worker` a mano con `Replay::none()` y un canal de drenaje que nadie dispara (`crates/clawg-core/src/eval_api.rs:482-489`) para medir con el mismo worker del daemon. Consecuencia de diseño: el outbox tiene que ser opcional por evento (un evento sin fila se comporta como hoy), o se rompen las pruebas y la medición. |
| README: «reintentos idempotentes con `operation_id` evitan duplicar un save reintentado» | **Parcial, y casi ya resuelto por el dedup.** Un save repetido hoy no duplica filas: responde `201 reinforced` con el id existente (`specs/ingest`, «Un duplicado devuelve la fila existente»). Lo que un `operation_id` agregaría es no reforzar de más, no resucitar una copia borrada y poder preguntar «¿qué pasó con mi save?». Ver «Idempotencia». |

## Problema (verificado contra el código)

### Qué se pierde y cuándo

- **Todo `202` vive sólo en RAM hasta que el worker lo procese.** `try_send` mete el evento en el `mpsc` (`api.rs:738-743`) y no hay otra copia hasta el volcado de un apagado ordenado. Dos clases de `202`: `embedder_loading` (el worker retiene los eventos con `hold_for_embedder`, `worker.rs:327-348`) y `unconfirmed` (la espera de 10 s venció o el worker soltó la respuesta).
- **La ventana de carga es la de cada arranque.** El daemon contesta `/health` y acepta saves mientras el modelo se descarga o carga (`serve.rs:84-98`); `clawg mcp` lo arranca en la primera herramienta y espera hasta `AUTOSTART_TIMEOUT` = 60 s (`client.rs:40`, `client.rs:969-998`). Después de un cambio de pooling, el re-embebido de un store viejo también deja el estado en `loading` (`specs/storage`, «Re-embebido de un store CLS al arrancar»). El repo no mide cuánto dura la carga ni con qué frecuencia se mata un daemon; el argumento es de contrato, no de frecuencia.
- **El orden de los caminos de salida.** Un apagado por `clawg stop` (el instalador lo usa antes de reemplazar el binario, `specs/storage`, «El instalador aborta si el daemon sigue vivo») vuelca bien. Un kill, un crash del proceso, SIGTERM (sin handler) o el cierre de sesión de Windows, no.
- **Matiz de la tesis.** Con el embedder listo, un kill durante la espera de 10 s corta la conexión: el agente recibe un error de transporte, no «aceptado». Lo perdido ahí no contradice lo que se le dijo, y su reintento lo absorbe el dedup. Lo que contradice lo que se le dijo son los `202`.

### Qué no llega nunca al agente

- Un fallo después del `202` (embed, insert) sólo se registra (`worker.rs:257-264`). El agente no tiene dónde verlo: `/health` no cuenta nada (`api.rs:409-426`), `memory_status` sólo imprime ese `Health` (`crates/clawg-cli/src/mcp/tools.rs:331-349`) y `list_memories` lista filas de `memory`, que un save fallido no tiene.
- No hay intentos ni causa guardados: un evento que mata el proceso al procesarlo (por ejemplo un panic en el embedder) se reinyectaría del volcado en cada arranque sin límite, y uno que falla queda perdido sin rastro.

### Un save largo y el replay

- Un fallo a mitad deja un prefijo (`specs/ingest`, «Un fallo del worker llega al que guardó»). Con un kill entre el insert del último pedazo y `link_save`, las filas quedan sin vincular. Un replay las reabsorbe como `Reinforced` y `link_save` sólo conoce la cabecera de las posiciones que insertó el evento actual (`worker.rs:587-594`), así que las vincula con `context` vacío. Pero `memory_piece.context` existe «so a re-embed can rebuild the same vector» (`crates/clawg-core/migrations/V11__memory_piece.sql:5-8`): un pedazo embebido tras su cabecera y vinculado sin ella se re-embebe distinto. Es un daño silencioso al reintentar, no al fallar.
- Además cada pedazo reabsorbido suma un `access_count` que el usuario nunca generó.

### Dónde está el texto sin enmascarar

- El worker enmascara (`worker.rs:430`); el handler no. Hoy sólo el volcado guarda texto crudo. Un outbox que copie el evento tal cual dejaría secretos de formatos conocidos dentro de `memory.db` y de todos sus snapshots (`specs/storage`, «Un snapshot es completo»), por una vía que `clawg secrets` (que lista filas de `memory`) y `clawg purge` (`crates/clawg-core/src/memory/secrets.rs:81-130`) no conocen.

## Lo que ya existe y se reutiliza

- Las piezas de seguridad de datos de la base: migraciones atómicas con checksums fijados (`PINNED`, `crates/clawg-core/src/memory/migrations.rs:44`), snapshot `pre-migration` antes de aplicar una pendiente (`db.rs:207-217`), base más nueva rechazada sin tocarla (`specs/storage`, «Verificación del esquema antes de migrar»). La última migración es V12; la de este cambio toma V13.
- El worker único y secuencial (`worker.rs:198-275`): el outbox no agrega escritor. La tabla es la fuente de verdad y el `mpsc` pasa a ser el aviso de que hay algo para procesar.
- `redact` es pura y su segunda pasada no encuentra nada nuevo (`crates/clawg-core/src/memory/redact.rs:131`; test `masking_twice_finds_nothing_new`, `redact.rs:232`). Se puede enmascarar en el handler y dejar el `redact` del worker como segunda pasada sin cambiar el texto.
- `MemoryEvent` ya sabe agregar campos opcionales con `#[serde(default)]` (`crates/clawg-core/src/memory/events.rs:32-43`): el JSON del evento puede viajar en la fila y crecer sin romper filas escritas por una versión anterior.
- El lector del volcado (`spill::load`, `parse_lines`, `REJECTED_FILE`: `spill.rs:62-94`, `122-132`) sirve tal cual para importar una vez el archivo viejo.
- `uuid` con `v4` ya es dependencia de `clawg-core` (`Cargo.toml` raíz, línea 22; `crates/clawg-core/Cargo.toml:34`): un id de operación no suma dependencia.
- `/health` y `Health` ya son aditivos: el cliente lee los campos nuevos con `#[serde(default)]` (`client.rs:115-130`), y `memory_status` imprime el `Health` entero (`tools.rs:331-349`) sin herramienta nueva.
- La misma medición de latencia del save que usó ingest-durability (`openspec/changes/archive/2026-10-12-ingest-durability/verify.md:33-47`): 100 saves secuenciales, corto (1 chunk) y largo (5 chunks), perfil dev con el modelo real. Línea base: corto 32 ms p50 y 51 ms p99, largo 546 ms y 702 ms (build debug).
- `new_for_test` corre las migraciones sobre una base en memoria (`db.rs:248-254`), así que la tabla nueva existe en todos los tests sin trabajo extra.

## Enfoques posibles

### A. Outbox en una tabla de `memory.db` (recomendado)

El handler, después de validar y de enmascarar, inserta una fila `ingest_outbox` y recién entonces encola y contesta. El worker procesa la fila y la borra **en la misma transacción que inserta la memoria**. Al arrancar, el worker lee las filas pendientes en orden de id antes que cualquier evento nuevo (el mismo orden que hoy impone el volcado, `specs/storage`, «Reinyección al arrancar»).

Forma candidata (a afinar en el diseño):

```
ingest_outbox(
  id INTEGER PRIMARY KEY AUTOINCREMENT,   -- orden FIFO
  operation_id TEXT NOT NULL,             -- UUID; UNIQUE (agent_id, operation_id)
  agent_id, project_id,
  event TEXT NOT NULL,                    -- MemoryEvent ya enmascarado, JSON
  redacted TEXT NOT NULL DEFAULT '',      -- tipos enmascarados, para el `201`
  state TEXT NOT NULL,                    -- pending | failed
  attempts INTEGER NOT NULL DEFAULT 0,
  last_error TEXT,
  stored_pieces TEXT,                     -- progreso de un save largo (ids y cabeceras)
  created_at, updated_at
)
```

- **Aceptar es durable.** La fila se confirma antes de la respuesta. `202` pasa a querer decir «hay una fila que se procesará o fallará a la vista».
- **Sin pérdida entre insertar y borrar.** Para un chunk, `DELETE` de la fila y `INSERT` de la memoria van en una sola transacción (hoy ya hay una por pieza: `store_insert.rs:351-399`). No puede quedar una memoria sin borrar su fila, ni una fila borrada sin su memoria.
- **Entrega doble sin efecto.** El worker empieza cada evento «reclamando» la fila: si ya no existe (se procesó por la lectura de arranque y también llegó por el canal), la salta. Evita el refuerzo de más que haría el dedup.
- **Intentos y fallo visibles.** `attempts` sube al tomar el evento (un commit chico por save; con WAL y `NORMAL` no hay fsync por commit, `db.rs:186-189`). Pasado un tope la fila queda `failed` con `last_error` y el worker sigue con la siguiente: un evento que mata el proceso deja de reinyectarse para siempre. Una falla transitoria (`IngestFailure::EmbedderUnavailable`, `worker.rs:47-48`) no cuenta como intento.
- **Enmascarado antes de persistir.** El handler llama a `redact` y guarda el texto enmascarado más los tipos; el worker repite su `redact` (no encuentra nada) y el `201` toma los tipos de la fila (`api.rs:652`). Reemplaza la decisión D2 de secret-hygiene sólo para la copia que se persiste.
- **Reemplaza al volcado.** El apagado ordenado deja de volcar: lo no ingerido ya está en la tabla. `ingest-pending.jsonl` se lee **una vez** al arrancar, se importa a la tabla (enmascarando cada evento) y se borra después de confirmar la transacción. `spill.rs` se reduce a ese lector.
- **A favor:** cubre kill, crash y señal de terminar sin depender de ningún handler; un solo almacén para el contenido y su estado; el estado `failed` y los contadores salen de la misma tabla; aprovecha la maquinaria de migraciones y snapshots que ya existe.
- **En contra:** una tabla nueva y una migración; dos commits chicos más por save; toca la ingesta, el apagado y varias specs; el texto pendiente vive en la base y en sus snapshots, lo que obliga a enmascarar y a enseñarle a `purge` la tabla.

### A.1. Cómo se guarda un save largo (a elegir dentro de A)

- **A.1a. Progreso por pieza (recomendado).** Cada insert de pieza actualiza `stored_pieces` de la fila en la misma transacción. La última pieza borra la fila y escribe el vínculo `memory_piece` (con sus cabeceras) en esa misma transacción. Un replay retoma desde la pieza siguiente, no refuerza las ya guardadas y vincula con las cabeceras originales. Hay que comprobar que el troceo del texto enmascarado da la misma cantidad de piezas; si no (cambió `tokenizer.json`), se cae al camino del dedup de hoy.
- **A.1b. Todo el save en una transacción.** Atómico de verdad, pero la detección de contradicciones (`detect_contradiction`, `crates/clawg-core/src/memory/contradiction.rs:137`) es `async` y consulta la misma conexión pieza a pieza: habría que mover dedup, detección e insert a un único closure síncrono dentro de un `conn.call`. Un cambio grande al corazón del worker por un beneficio que A.1a da con menos riesgo.

### B. Mantener la cola en RAM, con manejadores de señal y volcado periódico

Un manejador de SIGTERM en Unix, uno de cierre de consola en Windows y un volcado cada N segundos.

- **A favor:** sin esquema, sin migración; barato.
- **En contra, y lo que sigue perdiendo:** un kill duro, un crash del proceso, el cierre de sesión de Windows en un proceso sin consola (el daemon autoarrancado es `DETACHED_PROCESS`, `client.rs:868-873`: los eventos de consola no le llegan) y la ventana entre dos volcados. El volcado periódico reescribe el archivo entero con rename (`spill.rs:135-138`) y deja texto crudo en disco por más tiempo. Tampoco cura que un fallo posterior al `202` sea invisible.

### C. Persistir sólo mientras el embedder carga

El outbox se usa únicamente para el `202 embedder_loading`.

- **A favor:** menos filas y menos costo en el camino caliente.
- **En contra:** deja el `202 unconfirmed` (la espera de 10 s vencida) igual que hoy, y los fallos posteriores siguen invisibles. Dos caminos (con fila y sin fila) para el mismo evento duplican los estados que hay que probar. La ventana de carga es la mayor, no la única.

**Recomendación:** A con A.1a. B queda como endurecimiento aparte si se quiere reducir la reinyección, no como solución de durabilidad.

## Idempotencia: `operation_id`

- **Qué ya garantiza el dedup.** Un save repetido no crea otra fila (`specs/ingest`). La brecha es chica: refuerza de más y puede resucitar una copia borrada o reemplazada (`store_insert.rs:241-242`).
- **Quién reintentaría.** El puente MCP no reintenta solo (`crates/clawg-cli/src/mcp/server.rs:401-418` sólo traduce errores en texto): el reintento lo decide el modelo, que no tiene un id estable que repetir. Quien sí podría reenviar con el mismo id es un hook (cambio 25) con su plazo de 30 s.
- **Propuesta.** El daemon **siempre** emite un `operation_id` (UUID) y lo devuelve en el `202` y en el `201`. `SaveRequest` acepta uno opcional (`api.rs:311-323` no usa `deny_unknown_fields`: el campo es aditivo), con `UNIQUE (agent_id, operation_id)`: reenviarlo mientras la fila existe devuelve la misma respuesta de aceptado, sin una segunda fila. **No** se agrega a la herramienta `save_memory`: el modelo tendría que inventar un id único y el esquema pagaría tokens en cada sesión (ver «Cómo el agente se entera»).
- **Retención.** Mientras la fila esté `pending` o `failed`. Al completarse se borra; un reenvío posterior cae en el dedup y responde `reinforced`, que es lo que hace hoy. Guardar un registro mínimo de los completados (sin texto) permitiría preguntar por cualquier id, pero suma una tabla que crece y una pasada de limpieza; se deja como decisión.

## Cómo el agente se entera del resultado

| Opción | Tokens por sesión | Veredicto |
|---|---|---|
| **Campos en `/health` y `memory_status`** | Cero hasta que se llama a `memory_status` | **Recomendado.** `Health` suma un bloque `ingest` (`pending`, `failed`, edad del más viejo y los últimos errores con su `operation_id`), con `#[serde(default)]` en el cliente (`client.rs:115-130`). `format_status` ya lo imprime (`tools.rs:331-349`). Un daemon viejo no manda el bloque y un puente nuevo lo trata como «desconocido». |
| **Texto del `202`** | Cero | **Sí, junto con lo anterior.** Los dos textos de `save_outcome_text` (`server.rs:463-468`) incluyen el `operation_id` y dicen que si el save falla aparece en `memory_status`. |
| **Una herramienta nueva (`save_status`)** | Descripción y esquema en cada sesión | **No.** Los tests fijan 11 herramientas (`server.rs:1520` y la lista de `server.rs:1527-1549`) y quality-guards (32) versiona el contrato; una herramienta más sólo para consultar fallos poco frecuentes paga siempre. |
| **Un filtro en `list_memories`** | Un argumento más | **No.** Un save fallido no es una fila de `memory`, y `kind`/`id` de esa herramienta son exactamente lo que toma `delete_memory` (`specs/ingest`). |
| **Un aviso en `recall_context`** | Una línea cuando hay fallos | **Para después.** Es el lugar donde el agente mira al empezar, pero toca el presupuesto de 23 y el inyector de 25; el campo de `/health` les da el dato cuando los construyan. |

`/health` no debe esperar detrás del store: lo consulta `probe_running` antes de cada autoarranque con un tope de 1 s (`client.rs:44`, `client.rs:937-944`). Un `COUNT` agrupado sobre una tabla chica con índice por `state` cabe, pero es una segunda lectura de la misma conexión serializada (decisión abierta: tabla o contadores en memoria con el valor inicial leído al arrancar).

## Costo y durabilidad por save

- **Costo por save en A.** Una inserción de la fila (antes de contestar), un `UPDATE` de intentos al tomarla y el `DELETE` dentro de la transacción del insert que ya existe: **dos commits más**, más el texto del evento duplicado un instante en la base. Con `NORMAL` en WAL no hay fsync por commit.
- **Corte de luz.** Si la inserción del outbox debe sobrevivirlo, hay que sincronizarla: `PRAGMA synchronous = FULL` para esa inserción (la conexión es una sola y serializada, README «Ideas evaluadas», y `PRAGMA` se puede fijar fuera de la transacción) cuesta un fsync por save aceptado. Es una decisión del usuario entre kill/crash (gratis) y corte de luz (un fsync).
- **Cómo medirlo.** El mismo procedimiento de ingest-durability: 100 saves secuenciales, corto y largo, perfil dev, modelo real, antes y después, reportando respuesta y fila en p50, p95 y p99; una segunda corrida con `FULL` si se acepta como opción. Es un build debug (cota alta): el release necesita autorización.

## Descartado

| Opción | Razón técnica |
|---|---|
| Un archivo de registro (WAL propio) con fsync por save, sin tabla | Sin migración, pero no puede borrar su entrada en la transacción que inserta la memoria: la entrega pasa a «al menos una vez» y el replay refuerza filas (`access_count`) en cada crash. Además el estado (`attempts`, `failed`) obliga a reescribir el archivo. Hay que mantener dos almacenes que se desfasan, en un proyecto que eligió la transacción única memoria + vector + FTS. |
| Insertar en el handler, sin worker | Mete un segundo escritor: el dedup sólo es correcto con un consumidor secuencial (`api.rs:671-679`, `store_insert.rs:268-279`). |
| Subir `SAVE_CONFIRM_TIMEOUT` | Alarga la espera sin tocar lo que pasa si el proceso muere. |
| Un `operation_id` obligatorio en la herramienta `save_memory` | El modelo no tiene un id estable que repetir, y el esquema suma tokens a toda sesión. |
| Una herramienta `save_status` | Rompe el conteo fijado y el contrato versionado por un dato de uso raro. |
| Guardar el texto crudo y enmascarar en el replay | Deja secretos en `memory.db` y en cada snapshot durante todo el tiempo que la fila viva. |
| Borrar sola una fila `failed` vieja | La base del usuario no se destruye en silencio (`openspec/config.yaml:30`); un save que el agente dio por aceptado no se descarta sin que alguien lo decida. Un tope de filas `failed` que descarte las más viejas es lo mismo con otro nombre. |
| Un id de evento calculado en el puente (hash del contenido más la hora de la petición) | El puente no reintenta solo (`crates/clawg-cli/src/mcp/server.rs:401-418`). El reintento lo decide el modelo, y es otra llamada a otra hora, así que el id sale distinto y no deduplica nada que el dedup por `content_hash` no absorba ya. El id útil es el que emite el daemon y un llamador repite a propósito (ver «Idempotencia»). |
| Reintentos con espera creciente dentro de la misma ejecución | La única causa transitoria (`EmbedderUnavailable`, `worker.rs:47-48`) ya se retiene sin contar intento. Las demás (un embedding inválido, un insert que falla) se repiten igual con el mismo texto. Reintentar al arrancar, con tope de intentos, cubre lo que cubriría una espera. |
| Una lápida al borrar para que un replay no reviva la memoria | Como la fila se borra en la misma transacción que el insert, un evento completado no deja nada que reponer. Lo único que puede «revivir» es un save pendiente cuyo gemelo se borró antes de procesarse, y ese save es posterior al borrado (ver recoverable-delete en «Interacciones»). |

## Interacciones

- **agent-lifecycle-hooks (25).** Va a guardar desde hooks (por ejemplo `compact_summary` de `PostCompact`, opción A5 de su exploración) justo cuando las sesiones empiezan y el modelo puede estar cargando; el hook tiene un plazo de 30 s y no autoarranca. Este cambio tiene que cerrar antes: sin él, un `202 embedder_loading` desde un hook es una promesa en RAM. El `operation_id` opcional es lo que un hook que reintenta necesita.
- **doctor-checks (31).** Lee `ingest-pending.rejected.jsonl` y trata `ingest-pending.jsonl` como informativo (`doctor-checks/exploration.md:30`, `:129`). El archivo rechazado se conserva (el importador del volcado viejo sigue escribiéndolo). Pero 31 define sus chequeos como lecturas del directorio de datos que «nunca abren `memory.db`» (`:55`); un chequeo de filas `failed` o pendientes muy viejas necesita la lectura de sólo lectura de `probe` (`specs/storage`, «`clawg status` muestra el esquema sin daemon») o el `ingest` de `/health`. Hay que acordar cuál.
- **recoverable-delete (18).** Una fila de `memory` borrada no la reabsorbe el dedup (`store_insert.rs:241`): un save pendiente del mismo texto crea una fila nueva al procesarse. Es coherente (el save es posterior al borrado), pero 18 tiene que saberlo al definir qué hace un restore con el texto repetido.
- **purge.** `clawg purge <ID>` saca el texto de `memory.db` y sus WAL (`secrets.rs:81-130`) y nombra los snapshots que lo conservan mirando sólo `memory` (`secrets.rs:134-149`). Con el outbox, `purge` también tiene que borrar las filas pendientes o `failed` cuyo texto contiene el purgado, dentro de la misma transacción y antes del `VACUUM` y el checkpoint. Para eso la fila necesita el texto en una columna buscable (`instr`), no sólo dentro del JSON escapado. Los snapshots con filas pendientes se nombran como los que ya conservan la memoria.
- **Backups y restore.** Un snapshot incluye la tabla (`VACUUM INTO`, `specs/storage`). Hoy el volcado sobrevive a un `restore` porque vive fuera de la base (`openspec/changes/archive/2026-10-12-storage-safety-net/design.md:59`); con la tabla, un restore reemplaza las filas pendientes por las del snapshot, y las de la base anterior quedan sólo en el `pre-restore`. Decisión abierta: traspasarlas a la base restaurada o dejarlas en el `pre-restore` y decirlo.
- **topic-revisions (27).** Agrega `topic_key` a `MemoryEvent` con `#[serde(default)]` (`topic-revisions/exploration.md:45`, `:104`): como la fila guarda el evento serializado, el campo viaja sin esquema nuevo. Un binario anterior que lea una fila de uno nuevo pierde la clave pero no el texto; como un binario anterior rechaza una base más nueva, el caso no ocurre.
- **quality-guards (32).** Sólo cambian los textos de los resultados de `save_memory`, no el esquema de la herramienta. Si 32 ya fijó el contrato, el cambio de texto aparece como diff a propósito. Tampoco hay herramienta nueva.
- **secret-hygiene (archivado).** Se reabre la decisión D2 sólo para la copia persistida: el handler enmascara antes de escribir, y el worker conserva su pasada (`worker.rs:430`). El reporte `redacted` del `201` sale de la fila y no de la segunda pasada, que no encuentra nada.
- **context-budget (23) y memory-protocol (22).** El texto de `INSTRUCTIONS` (`server.rs:108-119`) no necesita cambios. Un aviso de saves fallidos en `recall_context` queda como consumidor futuro del bloque `ingest`.
- **install-smoke (40).** El test 1 de este cambio abre un segundo runtime sin apagado ordenado: es la prueba en proceso. La prueba con el binario real vive en 40: guardar con el embedder cargando, matar el proceso por su handle, volver a levantar `serve` y buscar. Hoy falla para un `202`, y pasa con este cambio.
- **bridge-reconnect (39).** Hasta que exista el `operation_id`, 39 sólo reintenta lo que no llegó al daemon (un 401 o una conexión rechazada) y nunca un timeout. Con el id, un save vencido se puede repetir sin efecto.
- **history-import (44).** Importa por el camino propio de memory-portability (43), no por la cola. Una importación grande no llena el outbox ni compite con los saves de la sesión.

## Migración de lo guardado

- **Qué hay hoy:** ninguna tabla de ingesta. Los eventos aceptados viven en RAM y, tras un apagado ordenado, en `ingest-pending.jsonl`.
- **Qué agrega:** una migración nueva, V13 (la última publicada es V12, `crates/clawg-core/migrations/V12__project_docs.sql`), con la tabla y sus índices. No toca ninguna tabla existente ni convierte datos: **no hay filas del usuario que migrar**. Se suma su fila a `PINNED` en el mismo commit (`migrations.rs:44`).
- **Al abrir una base en V12:** se aplica V13 en la transacción agrupada de siempre, con snapshot `pre-migration` antes (`db.rs:207-217`). Todas las memorias siguen alcanzables.
- **Volcado viejo:** si existe `ingest-pending.jsonl` al arrancar, el nuevo binario lo importa **una vez**: cada evento válido se enmascara y se inserta como fila en una sola transacción; el archivo se borra recién después del commit (si el borrado falla, se avisa y la segunda lectura se absorbe por dedup, con el refuerzo de más ya descrito). Las líneas ilegibles siguen yendo a `ingest-pending.rejected.jsonl` (`spill.rs:75-92`).
- **Camino de vuelta:** un binario anterior que abra una base con V13 la rechaza sin tocarla (`SchemaNewer`, `specs/storage`) y el mensaje dice cómo volver: instalar la versión que la escribió o `clawg restore` de un backup. Antes de bajar de versión, `clawg stop` con el binario nuevo drena el outbox; lo que quede `failed` no se descarta, pero no se reinyecta con el binario viejo.
- **Test de base sembrada (regla de `config.yaml:31`):** una base en V12 con memorias, vínculos de piezas y disputas, más un `ingest-pending.jsonl` con eventos válidos y una línea corrupta; abre con V13, todas las filas se alcanzan por búsqueda, los eventos del archivo quedan como filas del outbox y se procesan. Adapta `a_store_from_the_previous_version_opens_with_every_row_untouched` (`db.rs:737`).

## Alcance candidato

1. Migración V13 con la tabla del outbox y su fila en `PINNED`.
2. Función pura `OutboxDecision` (qué hacer con una fila al tomarla: procesar, saltar, marcar `failed` por tope de intentos) y pura la serialización de la fila, sin I/O.
3. Handler: enmascara con `redact`, inserta la fila (con `synchronous` según la decisión), encola y contesta; con la cola llena responde `503` y borra la fila.
4. Worker: reclama la fila, procesa, borra en la transacción del insert, anota `last_error` y `attempts` ante un fallo, y retoma un save largo desde `stored_pieces` (A.1a). La inserción con vínculo de piezas va en la misma transacción final.
5. Arranque: carga las filas pendientes en orden antes de cualquier evento nuevo; importa una vez el volcado viejo.
6. Apagado: deja de volcar; mantiene el plazo de drenaje y la relación con `STOP_TIMEOUT` (`crates/clawg-cli/src/stop.rs:19`).
7. `operation_id` emitido por el daemon y aceptado opcional en `SaveRequest`, devuelto en `201` y `202`.
8. Bloque `ingest` en `/health` y `Health`, impreso por `memory_status`; textos de `save_outcome_text` con el id.
9. `purge` borra las filas que contienen el texto; `backups_holding` nombra los snapshots con filas pendientes.
10. Docs: `CLAUDE.md` del repo (API HTTP, `ingest-pending.jsonl` ya no se escribe) y las specs `ingest` (timeout, embedder cargando, cola acotada) y `storage` (apagado sin pérdida, reinyección, formato del volcado).

### No-alcance

- Un manejador de SIGTERM o de cierre de consola (B): queda como endurecimiento, ya sin riesgo de pérdida.
- Una herramienta MCP nueva, un filtro nuevo de `list_memories` o un argumento nuevo en `save_memory`.
- Un comando `clawg outbox` para listar, reintentar o descartar filas `failed` (decisión abierta).
- Cambiar el chunker, el dedup, la detección de contradicciones, `DRAIN_DEADLINE` o la capacidad de la cola.
- Cambiar el ranking: no aplica `rag-eval`; la ruta de `EvalIngest` no usa el outbox.
- Retomar un bootstrap del embedder o cualquier cambio del contrato de embedding.

## Decisiones abiertas

Son del usuario.

- [ ] **Qué falla cubre el outbox.** ¿Kill, crash y señal de terminar (inserción con `NORMAL`, sin costo extra) o también corte de luz (`FULL` sólo para la inserción del outbox, un fsync por save aceptado, o `FULL` global)? Se decide con la medición de latencia.
- [ ] **Cola llena.** ¿`503 ingest_queue_full` y se borra la fila (recomendado: el contrato de `specs/ingest` no cambia) o la fila se conserva y el save contesta `202`?
- [ ] **Save largo.** ¿Progreso por pieza (A.1a, recomendado) o una sola transacción (A.1b)?
- [ ] **Intentos.** ¿Qué tope (propuesta inicial: 3) y qué cuenta como intento? La causa transitoria (`EmbedderUnavailable`) no cuenta.
- [ ] **Qué pasa con una fila `failed`.** Se queda hasta que alguien decida. ¿Basta con mostrarla en `memory_status` y en `doctor`, o se agrega un subcomando para reintentar y descartar?
- [ ] **`operation_id`.** ¿Sólo en HTTP y no en la herramienta (recomendado)? ¿Se retiene únicamente mientras la fila vive, o se guarda un registro de completados sin texto?
- [ ] **Cómo se entera el agente.** ¿`memory_status` y el texto del `202` (recomendado) o además `recall_context` (con 23 y 25)?
- [ ] **Contadores de `/health`.** ¿Un `COUNT` sobre la tabla o contadores en memoria inicializados al arrancar? El segundo puede divergir de la tabla.
- [ ] **`restore`.** ¿Las filas pendientes de la base reemplazada se traspasan a la restaurada o quedan en el `pre-restore`?
- [ ] **Importador del volcado viejo.** ¿Cuántas versiones se mantiene el lector de `ingest-pending.jsonl` antes de borrarlo?
- [ ] **Enmascarar en el handler.** ¿Se acepta reabrir la decisión de secret-hygiene sólo para la copia persistida, con el worker repitiendo su pasada?

## Cómo se mide

Tests RED primero (cada uno falla hoy):

1. Un save aceptado con el embedder `loading` deja una fila confirmada antes de contestar `202`; si se abre otro runtime sobre el mismo directorio **sin** pasar por el apagado ordenado, el save se ingiere. Hoy sólo existe el volcado del apagado (`runtime.rs:229-241`).
2. Lo mismo para `202 unconfirmed` (worker detenido y plazo de prueba de 100 ms, como `a_save_the_worker_does_not_confirm_in_time_answers_202_unconfirmed`, `crates/clawg-cli/tests/api.rs`).
3. Atomicidad: tras un save confirmado no queda fila; ante un insert que falla (embedder con NaN, como `NanAtEmbedder` en `worker.rs:1889`) la fila queda con `attempts` y `last_error` y no queda una memoria a medias sin su progreso.
4. Un fallo después de un `202` aparece en el bloque `ingest` de `/health` y en el texto de `memory_status` con su `operation_id` y la causa. Hoy sólo hay un `log::warn!` (`worker.rs:257-264`).
5. Enmascarado: guardar un token de formato conocido con el embedder `loading` y leer la tabla: el secreto no está en ninguna columna; tras procesar, el `201` sigue listando los tipos enmascarados.
6. Save largo: matar entre la pieza 2 y la 3 de 3; el replay retoma en la 3, no suma `access_count` a las dos primeras y deja el vínculo `memory_piece` con las cabeceras originales (adapta los tests de `worker.rs:1889-1917`).
7. Entrega doble (lectura de arranque y canal): la segunda se salta y no refuerza.
8. Evento venenoso: un evento cuyo procesamiento aborta sube `attempts` en cada arranque, pasa a `failed` en el tope y el worker procesa la fila siguiente.
9. Volcado viejo: un `ingest-pending.jsonl` con dos eventos válidos y una línea corrupta pasa a filas enmascaradas, el archivo desaparece y la línea corrupta queda en `ingest-pending.rejected.jsonl` (adapta `a_corrupt_spill_line_is_set_aside_and_the_rest_ingested`, `crates/clawg-core/tests/shutdown.rs`).
10. Cola llena: `503 ingest_queue_full` no deja fila, y `the_production_ingest_queue_holds_1024_saves` (`runtime.rs`) sigue pasando.
11. `operation_id` repetido mientras la fila existe: una sola fila y la misma respuesta; un `SaveRequest` sin el campo se comporta como hoy.
12. Un evento sin fila (la ruta de `EvalIngest`, `eval_api.rs:482-489`) se procesa igual que antes, byte a byte.
13. `purge` de una memoria borra las filas del outbox que contienen su texto, y una búsqueda del texto en `memory.db` y su WAL no lo encuentra tras el checkpoint (extiende los tests de `secrets.rs`).
14. **Base sembrada en V12** (regla de `config.yaml:31`), como se describe en «Migración»; más: el snapshot `pre-migration` existe (`db.rs:207-217`), el checksum de V13 está en `PINNED`, y una base en V13 abierta por el binario de V12 se rechaza sin tocarla (`a_newer_database_is_refused_untouched`, `db.rs`).
15. `/health` con y sin el bloque `ingest`: un cliente nuevo contra un daemon viejo no inventa datos, y uno viejo ignora el campo.

Latencia del save (obligatoria porque agrega escrituras al camino caliente): el procedimiento de `openspec/changes/archive/2026-10-12-ingest-durability/verify.md:33-47` antes y después. Umbral propuesto, a pre-comprometer: el p95 de un save corto no crece más de 20 ms con `NORMAL` (línea base: 32 ms p50, 51 ms p99, corto; 546 ms y 702 ms, largo); con `FULL`, se reporta y decide el usuario. `rag-eval` no aplica: no cambia el ranking.

## Riesgos

- **Un texto pendiente dentro de la base.** Mitigación: enmascarar antes de persistir (test 5) y que `purge` conozca la tabla (test 13). Una fila `failed` conserva su texto enmascarado hasta que alguien decida, lo que obliga a que `doctor` y `memory_status` la muestren.
- **Tocar muchos tests fijados.** La reinyección, el formato del volcado y el apagado tienen sus tests en `crates/clawg-core/tests/shutdown.rs` y `spill.rs` (`specs/storage`); hay que reescribirlos para la tabla sin perder los casos (línea corrupta, apagado durante la reinyección, plazo vencido).
- **Costo en el camino caliente.** Dos commits y, si se elige, un fsync por save. Se mide antes de fijar la opción de durabilidad.
- **`/health` detrás de una conexión serializada.** El sondeo de 1 s de `probe_running` no debe bloquearse detrás del worker; la decisión de contadores lo cubre.
- **Troceo no determinista entre ejecuciones.** El progreso por pieza supone que el mismo texto da las mismas piezas. Si cambia `tokenizer.json` entre un crash y el replay, la fila cae al camino del dedup (con los efectos ya descritos).
- **Resucitar una copia borrada.** Un save pendiente que se procesa después de borrar su gemelo crea una fila nueva (`store_insert.rs:241`). Coherente con el orden de los hechos, pero visible para 18.
- **Importar el volcado dos veces.** Si falla el borrado del archivo tras el commit, la segunda lectura se absorbe por dedup y refuerza de más. Aceptable y avisado; la decisión sobre cuánto mantener el importador cierra el caso.
- **El corte de luz sigue abierto bajo `NORMAL`.** La tesis habla de «power loss»; con la opción recomendada por costo no se cierra. Queda dicho en la decisión de durabilidad para no prometer de más.
- **Estado del repo:** el `CLAUDE.md` del proyecto no menciona el volcado, pero las specs `ingest` y `storage` sí; cualquier cambio de contrato HTTP (`operation_id`, bloque `ingest`) obliga a actualizar la API documentada.
