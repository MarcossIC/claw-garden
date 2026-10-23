# read-connections — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal | M | 37 knn-prefilter (baja el costo de cada escaneo) y el volumen real que trae 44; se coordina con 55 y 53 | exploración (borrador) |

**Tesis:** una operación larga sobre la única conexión de SQLite del daemon frena detrás de ella todo guardado, búsqueda, contexto y hook. El cambio le da a las lecturas su propia conexión (o un grupo chico), mantiene un solo escritor y deja de encolar trabajo de base de datos desde `/health`. Hoy, salvo el snapshot, ninguna lectura tiene conexión propia: `tokio-rusqlite` corre un hilo por conexión y atiende en orden de llegada, así que una lectura de milisegundos espera lo que dure lo que tenga delante, un refresco del índice de docs o una ráfaga de escrituras de un guardado largo. Nada de lo aceptado se pierde (el outbox lo garantiza); lo que se estira es el plazo: la confirmación de un guardado, los 3 s de un hook, el `/health`. Pero el cambio sólo se justifica si la espera se **mide** con un store de tamaño real después de knn-prefilter (37): si no aparece, el resultado correcto es cerrar sin código (enfoque D). Es de tamaño M: dos o tres conexiones, un reparto de las rutas en lectura y escritura, y un diseño de pruebas, porque la suite usa un store en memoria y una segunda conexión a `:memory:` es otra base.

## Premisas que se verificaron y cuáles no eran del todo ciertas

| Premisa | Resultado |
|---|---|
| `MemoryStore` envuelve una sola conexión compartida por todos los clones | **Cierta.** `pub conn: Connection` (`crates/clawg-core/src/memory/db.rs:148`), clonable porque es el extremo de un canal; el guardián `_lock` va en un `Arc` (`:160`). Se abre en `db.rs:191` (`Connection::open`) y no hay ninguna otra conexión de escritura ni de lectura en el daemon. |
| `tokio-rusqlite` atiende en orden y soltar el futuro no cancela el trabajo | **Cierta.** Un hilo por conexión (`tokio-rusqlite-0.7.0/src/lib.rs:377`, `event_loop` en `:412`) y un canal `crossbeam_channel::unbounded` (`:376`). `call_raw` manda la clausura **antes** de esperar la respuesta (`:297-304`): un `tokio::time::timeout` que vence deja el trabajo en la cola. |
| El comentario de WAL describe lo que pasa | **Corregido en `b1f22f3`.** Decía «one writer and concurrent readers», y con una sola conexión nunca hubo un lector concurrente de la base. Ahora dice lo que WAL da de verdad: un commit sin fsync y un snapshot que lee en su propia conexión sin frenar a la principal (`db.rs:196-199`). Las búsquedas, el contexto y los listados siguen sin lector propio. El comentario de la búsqueda (`crates/clawg-core/src/memory/search.rs:1228-1229`) dice «All arms share one connection, so nothing ran in parallel to lose»: lo reconoce. |
| Todo pasa por esa conexión | **Cierta, con matices.** Guardado (`outbox_record`, `crates/clawg-core/src/memory/outbox.rs:208`), búsqueda, contexto, listados, estadísticas, el vaciado del rastreador de accesos (`access.rs`, cada 30 s o a 500 ids), mantenimiento (24 h, `maintenance.rs:28`), índice de docs, síntesis y el `checkpoint` de apagado (`crates/clawg-core/src/runtime.rs:372`). El `VACUUM INTO` de `POST /v1/backup` ya no: desde `b1f22f3` corre en una conexión propia (`MemoryStore::snapshot`). El puente MCP no abre la base: llama al daemon por HTTP. Los `.conn` que aparecen en `crates/clawg-cli/src/mcp/server.rs` están después de `#[cfg(test)]` (`:1250`). |
| `snapshot` sólo tiene el llamador `POST /v1/backup` | **Cierta para `MemoryRuntime::snapshot`, con un matiz.** `POST /v1/backup` (`crates/clawg-cli/src/daemon/api.rs:1858`) es su único llamador, pero `snapshot_into` (`crates/clawg-core/src/memory/backup.rs:305`, `VACUUM INTO` en `:317`) corre también en el arranque, sobre la conexión única, antes de migrar (`db.rs:227-232`, motivo `PreMigration`) y antes de re-embeber (`crates/clawg-core/src/memory/bootstrap.rs:452`, `PreReembed`). Esos dos ocurren antes de que haya tráfico, así que no son un caso del problema. |
| El snapshot parcial de un `VACUUM INTO` cortado es un riesgo | **No, ya está resuelto.** Se escribe como `.db.tmp` y se renombra al terminar (`backup.rs:313-322`); shutdown-budget (55) lo registra igual. |
| Mover `VACUUM INTO` a una conexión propia corta es un arreglo directo aparte | **Hecho.** `b1f22f3`: `MemoryStore::snapshot` abre una conexión corta con `backup::open_existing` dentro de `spawn_blocking` y corre ahí el `VACUUM INTO`; el store en memoria de las pruebas, que no tiene archivo, sigue copiando por la suya. `ac53f56` limita a 64 MiB el WAL que deja un checkpoint (`journal_size_limit`). Este cambio parte de ahí (ver «Interacciones»). |
| `search` y `context` escriben en la base de forma inline | **Falsa.** Las lecturas de `search.rs` (antes de `:1412`), `prompt_builder.rs` (antes de `:1014`), `workspace.rs` (antes de `:671`), `pieces.rs` (antes de `:277`) y `synthesis.rs` (antes de `:390`, salvo `synthesis_core`) no tienen `INSERT`/`UPDATE`/`DELETE`. Los accesos viajan por el canal del rastreador (`api.rs:1464`, `access.rs`); `last_accessed_at` lo escribe el vaciado, no la búsqueda. `regenerate_auto_section` (`workspace.rs:107-143`) lee la base y **escribe un archivo**, no una fila. Lo único que escribe una ruta «de lectura» es `POST /v1/synthesis`: sella `synthesis_review` en la misma transacción (`synthesis.rs:236-263`). |
| rusqlite pone `busy_timeout` de 5 s en cada conexión que abre | **Cierta.** `sqlite3_busy_timeout(db, 5000)` (`rusqlite-0.37.0/src/inner_connection.rs:119`). Un lector no falla de inmediato con `SQLITE_BUSY`. |
| En WAL, lectores y escritor no se bloquean y sólo un `wal_checkpoint(TRUNCATE\|RESTART\|FULL)` espera a los lectores | **Cierta, con un matiz que cambia el apagado.** Según la documentación de SQLite que viene en el SQLite incluido (3.50.2, `libsqlite3-sys-0.35.0/sqlite3/sqlite3.c:10185-10235`): FULL, RESTART y TRUNCATE invocan el manejador de ocupado hasta que no haya escritor y todos los lectores lean la última instantánea, y mientras esperan **bloquean a los escritores nuevos**; los lectores nuevos siguen. Lo que espera es un lector **con una lectura en curso**, no una conexión ociosa: un lector sin transacción abierta no sostiene instantánea. La premisa «hay que soltar los lectores antes del TRUNCATE final» es más estricta de lo necesario: hay que asegurar que **no queda una lectura en vuelo**. |
| La última conexión que cierra limpia el WAL con un bloqueo exclusivo | **Cierta** (`sqlite3.c:3368-3371`): cuando el número de conexiones baja de 1 a 0, la última toma un bloqueo exclusivo, hace checkpoint y borra el WAL; una conexión nueva que intente leer en ese momento falla con `SQLITE_BUSY`. Importa para que el cierre de los lectores y del escritor tenga un orden. |
| Cuando WAL no está disponible, el sistema sigue andando | **Cierta y cambia el diseño.** `open` sólo avisa y sigue con el diario de reversa (`db.rs:205-215`; el caso nombrado es un home en una unidad de red). En ese modo un lector **sí** choca con el escritor: un `COMMIT` pide el bloqueo exclusivo y los lectores reciben `SQLITE_BUSY` (hasta 5 s de espera). El lector separado sólo tiene sentido si `PRAGMA journal_mode` devolvió `wal`. |
| Una segunda conexión necesita lo mismo que la primera | **Cierta, con menos de lo supuesto.** `sqlite-vec` se registra con `sqlite3_auto_extension` (`db.rs:127-135`): vale para **toda** apertura posterior del proceso, sin hacer nada. Las UDF (`register_udfs`, `db.rs:377-382`: `content_hash`, `compute_tier`, `demote_tier`) las usan sólo escrituras: `compute_tier` en el vaciado de accesos (`access.rs:207`), `demote_tier` en mantenimiento (`maintenance.rs:46`), `content_hash` en migraciones y en el alta. Ninguna consulta de lectura las llama, así que un lector no las necesita (registrarlas cuesta nada). `SQLITE_LIMIT_VARIABLE_NUMBER` ya no se sube en ninguna apertura (`cde63af`): el SQLite incluido compila 32766. `memory.lock` ya lo tiene el proceso (`StoreLock`), así que abrir más conexiones dentro del proceso no lo toca. |
| Una lectura en la conexión del lector ve un guardado confirmado | **Cierta por construcción.** En WAL una transacción de lectura que empieza después de un `COMMIT` lo ve. Un `201` sale después del `COMMIT` de la inserción (el worker responde al terminar), así que la búsqueda siguiente lo encuentra. Un guardado **sin confirmar** (`202 unconfirmed`) puede no verse todavía, igual que hoy: la búsqueda ya puede correr antes de que el worker inserte. |
| El dedup y las reglas de `topic_key` dependen de un solo escritor | **Cierta, y más de lo que parece.** Ni `idx_memory_dedup` (`crates/clawg-core/migrations/V6__content_hash.sql:14-16`) ni `idx_memory_topic` (`V16__topic_key.sql:13-15`) son `UNIQUE`: la migración dice que la unicidad la asegura el worker de ingest por ser «the single sequential consumer» (`V6__content_hash.sql:9-13`). Lo deciden las transacciones de escritura del worker (`store_insert.rs`, `topic.rs`); ninguna lectura. Quedan en el escritor. Un matiz: los chequeos de `derived_from` del handler (`api.rs:1303-1324`) son lecturas anteriores al outbox y hoy ya no son atómicas con la inserción; pueden ir al lector sin cambiar nada. |
| `/health` sólo espera 250 ms | **Cierta, y el trabajo igual queda.** `tokio::time::timeout(HEALTH_INGEST_WAIT, outbox_summary())` (`api.rs:1141`, `:1149`) abandona la espera, pero la consulta ya está en la cola de la conexión (mismo mecanismo que arriba) y corre cuando nadie la espera. El cliente manda `/health` con un plazo de 1 s (`HEALTH_TIMEOUT`, `crates/clawg-cli/src/client.rs:50`, `:973`) y lo manda mucho: `probe_running` (`:1477-1481`), el sondeo de arranque (`:1529`), `clawg stop` (`stop.rs:27`), el hook `session-start` (`hook.rs:238`) y el viaje de `memories`/`status`. Con el hilo atrasado, cada uno suma una consulta. |

## Problema (verificado contra el código)

### 1. La cola es de una sola conexión y es FIFO

Todo `store.conn.call(...)` va a un canal sin tope y lo ejecuta un solo hilo (`tokio-rusqlite-0.7.0/src/lib.rs:376-377`, `:412`). Una operación larga no se puede intercalar: las que llegan después esperan su turno completo. Quienes la ocupan hoy:

- Los refrescos del índice de docs (`project_docs::refresh`, desde `search_docs`, `api.rs:2082`, y desde el registro de proyecto, `api.rs:1966`): muchas transacciones de escritura seguidas.
- El bucle de piezas de un guardado largo: un `UPDATE` de `progress` por pieza, más las inserciones y el KNN de contradicción (lo acota save-bounds, 54).
- El vaciado del rastreador y el mantenimiento (`access.rs`, `maintenance.rs`): cortos, pero también esperan y hacen esperar.
- `POST /v1/synthesis`, que lee varias consultas y sella `synthesis_review` en una transacción.

Una medición sintética fuera del repo, con el SQLite empaquetado (3.50.2), 50 000 filas y 111 MB, dio: un `VACUUM INTO` de 636 ms, y una consulta trivial encolada 5 ms después en la misma conexión esperó 631 ms; la misma consulta en una segunda conexión tardó 1,3 ms, y un `INSERT` en esa segunda conexión mientras el `VACUUM INTO` mantenía su transacción de lectura terminó en 2,9 ms. Sirve de orden de magnitud: la espera en la cola es del tamaño de la operación que la precede, no del costo de la consulta, y WAL ya deja que el escritor avance mientras otra conexión lee. La medición propia, con el perfil de datos y el modelo real, es el primer paso (ver «Cómo se mide»).

### 2. Quién paga la espera

- **El guardado.** `accept` escribe la fila del outbox antes de contestar (`runtime.rs:225` → `outbox_record`, `outbox.rs:208`): esa escritura espera su turno en la cola. Después el handler espera el informe del worker hasta `SAVE_CONFIRM_TIMEOUT` = 10 s (`api.rs:106`, `:1384`); si la fila del outbox ya tardó en entrar, el plazo corre sobre un trabajo que sólo empezó después. El guardado no se pierde (responde `202 unconfirmed`), pero el agente ve incertidumbre sin motivo propio.
- **El hook.** `clawg hook session-start` tiene 3 s en total (`HOOK_DEADLINE`, `hook.rs:26`) y manda en paralelo `recent_context`, `health` y el registro de sesión (`hook.rs:238`). Si vence, imprime **nada** y sale con 0: la sesión arranca sin memoria y sin aviso. El `context` sin consulta no embebe, y entonces toda su espera es la de la cola.
- **La búsqueda y el contexto.** Varias llamadas a la conexión por petición (coincidencias de identificadores, un KNN por tabla, candidatos léxicos y puntaje, `save_links`; `search.rs:1229`, `:514`; el contexto suma `regenerate_auto_section`, `build_*_prompt`, `unreviewed_count` y `conclusions_to_review_count`, `api.rs:1511-1563`). Cada una vuelve a hacer cola.
- **`/health`.** Ver la última fila de la tabla de premisas: cada sondeo agrega una consulta que corre cuando nadie la espera, justo cuando el hilo está atrasado. Es lo que hace que un hilo ocupado se atrase más.

### 3. Clasificación de cada ruta

El puente MCP y los hooks son clientes HTTP del daemon, así que se clasifican las rutas (`api.rs:1027-1055`); cada herramienta MCP cae en la de su ruta.

- **Lectura pura de la base:** `POST /v1/search` (más el canal del rastreador, que escribe después y por su cuenta), `POST /v1/context` (escribe el archivo del workspace, no la base), `GET /v1/memories`, `GET /v1/stats`, `GET /v1/memories/{kind}/{id}`, `GET /v1/memories/{kind}/{id}/save`, `GET /v1/disputes`, `GET /health` (`outbox_summary`).
- **Lectura y luego escritura:** `POST /v1/synthesis` (sella `synthesis_review`), `POST /v1/projects/docs/search` (lee, y si el índice tiene más de un minuto, lo refresca), `POST /v1/projects` (registro y, con `index_docs`, indexado), `PATCH` / `DELETE` / `restore` de memorias (leen el estado y escriben), `POST /v1/disputes/judge` y `/resolve`.
- **Escritura:** `POST /v1/memories` (`outbox_record`; antes, los chequeos de `derived_from`, que son lecturas), `POST /v1/backup` (largo; lee todo), `POST /v1/sessions` (sólo memoria del proceso: no toca la base), `POST /v1/shutdown`.
- **Fuera del daemon:** el vaciado de accesos y el mantenimiento escriben desde sus propias tareas; el worker de ingest lee y escribe en la misma transacción (dedup, `topic_key`, contradicción) y por eso se queda en el escritor entero.

## Lo que ya existe y se reutiliza

- `open_for_probe` (`backup.rs:528-538`) ya sabe abrir una conexión de sólo lectura con `SQLITE_OPEN_READ_ONLY | SQLITE_OPEN_NO_MUTEX` para `clawg status` y `clawg secrets` (`backup.rs:513`, `secrets.rs:40`). Es el precedente, pero **no sirve tal cual** para el daemon: abre con `immutable=1` cuando no hay `-wal` (para no dejar `-wal`/`-shm`), y con un daemon andando y datos sin checkpoint eso leería un archivo que cambia. El lector del daemon necesita la apertura común de sólo lectura sobre WAL.
- `MemoryStore::open` (`db.rs:184`) es el único sitio donde nacen las conexiones; ahí se agrega el lector, con el modo de diario ya leído (`:197-205`).
- `PRAGMA query_only = ON` ya se usa en un test de mantenimiento (`maintenance.rs:1822`): refuerza el sólo lectura aunque se abra con permiso de escritura.
- `ensure_vec0_registered` (`db.rs:127`) ya vale para cualquier apertura posterior; no hay que registrar la extensión por conexión.
- El patrón «cada llamada es una función `async` sobre `&MemoryStore`» (`search_all_memories`, `build_memory_prompt`, `list_memories_core`, …) hace que mover una lectura sea cambiar de qué conexión sale su `call`, sin tocar a los llamadores.
- `AppCtx.save_timeout` (`api.rs:95`) y `HEALTH_INGEST_WAIT` (`api.rs:1141`) permiten acortar plazos en una prueba.
- `crates/clawg-cli/tests/binary.rs` levanta el binario real con `CLAWG_TEST_HOME` y `CLAWG_TEST_EMBEDDER=fake`, y tiene el caso de `session-start` (`binary.rs:1133`) para medir contra el hook real.
- El snapshot ya es atómico por `.db.tmp` + `rename` (`backup.rs:313-322`) y corre en su propia conexión de vida corta (`MemoryStore::snapshot`, `b1f22f3`): es el precedente de una segunda conexión dentro del daemon, con WAL leyendo una instantánea mientras el escritor sigue.

## Enfoques posibles

### A. Una conexión de sólo lectura (o un grupo chico) para las lecturas puras; toda escritura queda en la actual (recomendado, si la medición lo pide)

`MemoryStore` gana un lector abierto en `open` cuando el diario es WAL, con `SQLITE_OPEN_READ_ONLY` más `PRAGMA query_only = ON`, y un método que devuelve el lector o, si no hay, la conexión de escritura. Las funciones de lectura pura de la tabla pasan a usarlo. Las de lectura y escritura, y todo lo del worker, se quedan en el escritor. `outbox_summary` va al lector.

- **A favor:** quita de raíz que una lectura espere a una escritura o a un `VACUUM INTO`; sin cambiar el modelo de escritura (un solo escritor: el dedup y `topic_key` siguen siendo seguros); el costo es una conexión más y su caché de páginas. El hilo del lector también atiende varias consultas en orden, pero ahí cada una dura milisegundos.
- **En contra:** hay que decidir uno o varios lectores (uno serializa las lecturas entre sí; un grupo las paraleliza a costa de CPU que hoy comparte con el embedder). Un lector con una lectura larga **retrasa el checkpoint** y el `TRUNCATE` del apagado. Cada función que se migra debe ser realmente de sólo lectura, o falla con «attempt to write a readonly database»: eso es una red de seguridad, no un riesgo.
- **El modo sin WAL:** si `journal_mode` no es `wal`, no se abre lector y todo sigue como hoy. Se registra un aviso.

### B. Una conexión, pero partir las operaciones largas

La API de copia en línea de SQLite (`sqlite3_backup_step`) permite copiar por tramos. **No sirve para este caso**: el feature `backup` de rusqlite no está habilitado (`Cargo.toml:32`) y `Backup<'a, 'b>` toma prestada la conexión (`rusqlite-0.37.0/src/backup.rs:174-178`), así que no puede vivir entre dos `call`; para ceder el hilo entre tramos habría que usar FFI crudo sobre el puntero de la conexión. Lo que sí se puede es dar todos los pasos dentro de un `call`, que no libera el hilo. Se descarta como solución general; el único caso en que sirve, el backup, se resuelve mejor con una conexión corta propia (ver «Interacciones»).

### C. `/health` sin encolar trabajo

Tres formas, que no son excluyentes con A:

- **C1. Contadores en memoria** (un `watch` o atómicos que actualizan el worker y `accept`). Cero consultas en `/health`. El riesgo es que se desincronicen de la tabla si una ruta se olvida (un `outbox_forget`, una importación del `ingest-pending.jsonl` viejo, un reintento tras reinicio): dos fuentes de verdad.
- **C2. Un solo vuelo** (una bandera: no se encola otra cuenta mientras hay una pendiente; `/health` responde con la última cuenta conocida o sin ella). Una consulta como máximo, pero sigue en la cola del escritor si no hay lector.
- **C3. Nada: la consulta va al lector (A).** Con un lector, la cola de `outbox_summary` es la del lector y no tiene a nadie delante.

Recomendado: **C3 sola** si se hace A, porque no agrega estado; **C2** si no se hace A (cierra el crecimiento de la cola sin dos fuentes de verdad). C1 queda para si 53 mueve los contadores a memoria por otra razón.

### D. Sólo la medición

Medir, y si no hay espera medible, cerrar sin código y dejar la prueba como regresión. Es el resultado esperable si, después de 37, el escaneo es corto y la única operación larga (el backup) queda en su conexión propia. No cuesta nada y es el primer paso de todos modos.

### Recomendación: D como puerta, luego A con C3

1. **Medir** (ver «Cómo se mide»). Si la espera no se ve, cerrar con D.
2. **A con un lector** (no un grupo): todas las lecturas puras, incluida `outbox_summary`. Un grupo sólo si la medición muestra que las lecturas se esperan entre sí.
3. **C3** para `/health` (cae solo con A). **C2** únicamente si A se descarta y el `/health` abandonado sigue creciendo la cola.
4. **Pruebas:** `new_for_test` no cambia (ver «Decisiones abiertas»): sin lector, las lecturas caen al escritor; los RED que prueban el reparto usan un store en un directorio temporal.

## Descartado

| Opción | Razón técnica |
|---|---|
| Un grupo grande de conexiones de escritura | SQLite admite un solo escritor a la vez; más conexiones de escritura sólo pelean por el bloqueo y traen `SQLITE_BUSY` (5 s de espera). El dedup y las reglas de `topic_key` descansan en una escritura serializada. |
| Caché compartida (`cache=shared`) para que los tests usen `:memory:` con dos conexiones | La caché compartida usa bloqueos por tabla (`SQLITE_LOCKED`, que el manejador de ocupado no espera) y no es WAL; cambia el comportamiento que se quiere probar. |
| Copia en línea (`sqlite3_backup_step`) para partir el backup | Ver B: el `Backup` no puede cruzar dos `call` y el feature no está habilitado. |
| Un `Mutex` sobre una conexión síncrona compartida | Es lo mismo que el canal actual con peor contabilidad, y bloquea hilos del runtime de tokio. |
| Mover el índice de docs a otra base | Cambia el almacenamiento y el apagado sin que el problema lo pida; con un lector las consultas no esperan a su escritura de todos modos. |
| Contadores de `/health` en memoria como única solución | Dos fuentes de verdad que se pueden separar (C1); no ayuda a la búsqueda ni al contexto. |
| Alargar `SAVE_CONFIRM_TIMEOUT` o `HOOK_DEADLINE` | Oculta la espera; el hook tiene 3 s por el cliente, no por la base. |

## Interacciones

- **knn-prefilter (37).** Baja el costo de cada escaneo del KNN y debe entrar primero: sin eso se mediría el costo de la consulta, no el de la cola. Además cambia qué consultas hay (`search.rs`), y A mueve de conexión las mismas funciones. Conviene que 37 termine antes de migrar las lecturas.
- **history-import (44).** Trae el volumen real contra el cual medir y un pico de escrituras (miles de guardados) durante el cual las lecturas se esperarían. Es el motivo para tener el lector listo y la medición hecha antes de importar.
- **shutdown-budget (55).** Cambia el apagado. Con A hay que fijar el orden: terminar la fase HTTP (y con el `HTTP_GRACE` de 55, **una petición abandonada puede dejar una lectura en vuelo en el lector**, que retrasa el `TRUNCATE` hasta 5 s por el manejador de ocupado), después el drenaje, después `checkpoint`, y por último soltar los lectores y el escritor. Un lector ocioso no estorba, pero conviene que el apagado cierre el lector antes del `checkpoint` o cuente con una lectura en vuelo en el presupuesto. 55 debe contar ese peor caso.
- **outbox-lifecycle (53).** Si mueve los contadores de `/health` a memoria (C1), A ya no hace falta para `/health`. Si no, `outbox_summary` va al lector y 53 sigue dueño de la tabla. Las dos tocan `outbox.rs`, pero en funciones distintas.
- **Arreglo directo del `VACUUM INTO` en conexión propia (hecho, `b1f22f3`).** Era la mitad barata del problema (el backup dejó de ocupar el hilo, sin cambiar comportamiento) y entró antes y por separado. Este cambio **no lo duplica**: parte de que `snapshot` ya abre su conexión corta y mide lo que queda sin él (refrescos del índice, guardados largos, ráfagas). Si con ese arreglo hecho la medición ya no muestra espera, la conclusión es D.
- **doctor-checks (31), `clawg status` y `clawg secrets`.** Ya abren una conexión de lectura propia con `open_for_probe` (`backup.rs:528`, `:479`, `secrets.rs:40`) cuando el daemon está parado o sobre snapshots; no cambian. Con un daemon andando, `status` va por HTTP y no abre la base. Un chequeo de doctor que mire invariantes con el daemon andando tendría un lector natural; no es de este cambio.
- **query-plans (56).** Hermano de esta exploración: abarata las consultas del contexto. Se complementan: un plan mejor baja la duración de cada lectura, el lector evita que espere a una escritura. Conviene que 56 se mida antes, porque si el contexto queda en microsegundos, su espera en la cola pesa menos.
- **save-bounds (54).** Acota la duración de un guardado largo en el único hilo de escritura; este cambio no depende de eso pero se mide mejor con 54 entregado.
- **Specs:** `openspec/specs/storage/spec.md` (una conexión de escritura y las de lectura, el orden del cierre) y, si cambia `/health`, la de ingest. No cambia el contrato de `/health`, sólo cómo se calcula.

## Alcance candidato

1. Una prueba de medición (la puerta) y su decisión por escrito: seguir o cerrar con D.
2. Un lector de sólo lectura en `MemoryStore::open`, abierto sólo si el diario es WAL, con `query_only = ON`, y un método que devuelve el lector o el escritor.
3. Las lecturas puras de la clasificación pasan al lector: `search_all_memories`, el contexto (`build_*_prompt`, `regenerate_auto_section`, los dos conteos), `list`, `get`, `read_save`, `stats`, `open_pairs` y `outbox_summary`.
4. El cierre ordenado: lector antes del `checkpoint` (o su lectura en vuelo contada en el presupuesto de 55) y comprobación de que la última conexión no deja el WAL.
5. Aviso de registro cuando no hay lector (sin WAL).
6. Spec: delta de `specs/storage`.

### No-alcance

- Más de un escritor.
- El arreglo de `VACUUM INTO` en conexión propia (cambio directo aparte).
- Partir un backup, un refresco del índice o un guardado largo en pasos.
- Contadores de `/health` en memoria (53, si lo decide).
- Un grupo de lectores, salvo que la medición lo pida.
- Una transacción de lectura única para toda una búsqueda (ver decisiones).
- Tocar el modelo de consistencia: un guardado confirmado se ve en la búsqueda siguiente; uno sin confirmar, como hoy, puede no verse.
- Cambios de ranking: no se corre `rag-eval`.

## Decisiones abiertas

Son del usuario.

- [ ] **Umbral de la puerta.** ¿Qué espera medida justifica el cambio? Candidato: que, con el store de tamaño real y sin el `VACUUM INTO` en el hilo, la espera de un guardado con confirmación o la de `session-start` supere una fracción apreciable de su plazo (por ejemplo, un tercio de los 3 s del hook) durante una operación larga que el uso real produce. Si no, cierre con D.
- [ ] **Uno o varios lectores.** Uno (recomendado: simple, las lecturas son cortas), o un grupo de N (paraleliza lecturas a costa de memoria y de CPU compartida con el embedder).
- [ ] **Tests.** `new_for_test` (546 usos) sigue en memoria y las lecturas caen al escritor (recomendado: no toca la suite), o pasa a un archivo temporal con lector (cobertura del reparto en toda la suite, a costa de tiempo y disco). Costo de lo primero: sólo los tests con directorio temporal ejercen el lector.
- [ ] **Sin WAL.** ¿Se usa el escritor para todo y se avisa (recomendado), o se abre el lector igual y se acepta el `SQLITE_BUSY`?
- [ ] **Instantánea de una búsqueda.** Hoy cada llamada de una búsqueda es su propia transacción (otra escritura puede colarse entre dos). ¿Se mantiene, o se agrupa la búsqueda en una sola transacción de lectura (instantánea consistente, a costa de retrasar el checkpoint mientras dura)?
- [ ] **`derived_from` en el lector.** Los chequeos del handler van al lector (recomendado: ya no son atómicos con la inserción) o se quedan en el escritor para no discutirlo.
- [ ] **`/health`.** C3 (recomendado, si hay A), C2 si A no se hace, o C1 si 53 mueve los contadores.
- [ ] **Orden del cierre.** ¿El lector se cierra antes del `checkpoint` (recomendado) o se espera a que termine cualquier lectura en vuelo dentro del presupuesto de 55?

## Cómo se mide

**Puerta primero (sin código de producción).** Una prueba en `crates/clawg-cli/tests/binary.rs` (corre el binario real con `CLAWG_TEST_HOME` y `CLAWG_TEST_EMBEDDER=fake`) que siembra un store de decenas de MB, corre una operación larga (refresco de un índice de docs grande, un guardado de muchas piezas) y mide, mientras tanto: la latencia de `POST /v1/memories` con espera, la de `clawg hook session-start` (que además deja ver si imprime nada por vencer los 3 s) y la de un `POST /v1/search`; más la **profundidad de la cola** (un contador de llamadas en vuelo por conexión) con un `/health` cada 100 ms. La medición sintética de arriba es un orden de magnitud, no un dato del producto; la propia, con el modelo real y un perfil dev autorizado, confirma. El resultado fija si se sigue (A) o se cierra (D).

Después, tests RED:

1. **Una lectura no espera al escritor (determinista).** Con un store en directorio temporal, el hilo del escritor ocupado por una llamada bloqueada en un canal: una búsqueda por el lector contesta. Hoy no contesta hasta que se libera el canal.
2. **`/health` no suma trabajo.** Con una cuenta pendiente, otro `/health` no encola otra consulta (contador de llamadas en vuelo). Hoy cada uno agrega una.
3. **Consistencia.** Un guardado confirmado con 201 es visible a la búsqueda siguiente por el lector. Y un lector abierto antes de un `COMMIT` no ve el cambio hasta su próxima lectura (documenta la regla de instantánea).
4. **Sólo lectura.** Una función de lectura migrada que intenta escribir falla con error de sólo lectura (red de seguridad contra un reparto equivocado).
5. **Sin WAL.** Con un diario que no es `wal`, no hay lector y las lecturas caen al escritor sin error.
6. **Cierre.** Con una lectura en vuelo, el `checkpoint` del apagado queda dentro del presupuesto o informa lo que no pudo (hoy `runtime.rs:372-389` avisa «blocked by another connection»); sin lectura en vuelo, el WAL queda en cero.
7. **Reparto.** Una prueba pura que lista, por función, si usa lector o escritor, para que una función de escritura nueva no entre en el lector por descuido.

## Riesgos

- **Que la espera no exista.** Si, después de 37 y del `VACUUM INTO` aparte, no se ve, el cambio no se hace. Es un resultado válido, no un fracaso.
- **Una lectura larga del lector retrasa el checkpoint.** Sin el checkpoint automático el WAL crece mientras dure; y el `TRUNCATE` del apagado espera hasta 5 s por el manejador de ocupado. Se acota con lecturas cortas y con el orden del cierre.
- **Un grupo de lectores compite por CPU con el embedder.** Un solo lector evita el problema; un grupo se mide.
- **WAL no disponible** (unidad de red): el lector se desactiva. Quien está en ese caso sigue como hoy.
- **Una función de «lectura» que escribe** sale con error de sólo lectura en tiempo de ejecución. La prueba de reparto (7) y `query_only` lo detectan; la revisión de cada función es parte del diseño.
- **Consistencia entre llamadas de una búsqueda.** No empeora respecto de hoy (el escritor ya intercala escrituras entre las llamadas), pero deja de ser «casi siempre ordenado»: con un lector las escrituras avanzan en paralelo. Si importa, la instantánea única (decisión abierta).
- **Estado de `synchronous`.** `durably` (`outbox.rs:183-194`) sube y baja `synchronous` **por conexión**; el lector no escribe, pero cualquier función de escritura que se mueva por error hereda otra configuración. Otra razón para el sólo lectura.
- **Coordinación con 55 y 53.** El apagado y `/health` cambian en los tres; conviene diseñarlos con el mismo orden de cierre.
