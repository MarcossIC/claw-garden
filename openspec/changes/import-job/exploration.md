# import-job — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 (guardar memorias de forma eficiente) | M | import-linear (81) e import-decision-pass (82), que acortan lo que hay que seguir; import-undo (84) para el número de la importación; se coordina con read-connections (57), pipeline-metrics (73) y embed-throughput (85) | exploración (borrador) |

**Tesis:** `clawg import` manda el archivo entero dentro del cuerpo de un solo `POST /v1/import` y espera la respuesta sin plazo, sin progreso y sin forma de cancelar; en la importación real de 3.814 notas fueron 51 minutos de silencio. El cambio convierte la importación en un trabajo del daemon: `POST /v1/imports` contesta 202 con un id, `GET` informa la fase y los contadores, `DELETE` cancela en el límite de una nota y `clawg import --follow` lo muestra. Además el daemon lee el archivo desde una ruta que el cliente local nombra, en vez de recibirlo escapado en JSON, lo que quita varias copias del archivo en memoria de los dos lados; y el trabajo sostiene el modelo mientras lo necesita, para que el apagado por inactividad no lo suelte entre dos fases. Es posterior a la importación de producción: no la bloquea, y 81 y 82 le dejan al trabajo fases cortas que informar en lugar de un bloqueo de 800 s sin forma de verse desde fuera.

## Premisas que se verificaron y cuáles no eran del todo ciertas

| Premisa | Resultado |
|---|---|
| El cliente manda todo el archivo en un cuerpo y no tiene plazo | **Cierta.** `run` lee el archivo con `read_to_string` (`crates/clawg-cli/src/import.rs:59`) y `DaemonClient::import` arma un `json!` con `text` y lo serializa con `serde_json::to_vec` (`crates/clawg-cli/src/client.rs:1325-1329`, `:744-745`). Los clientes HTTP se construyen sin `.timeout(...)` (`client.rs:853`, `:860`) y una prueba fija que sólo `health` pone plazo (`client.rs:1784-1791`). Sólo se rechaza antes de enviar un archivo mayor que `MAX_IMPORT_BYTES` = 256 MiB (`import.rs:56`, `crates/clawg-core/src/memory/portability/import.rs:51`). |
| El handler recibe el archivo como un campo `text` y espera toda la importación | **Cierta.** `import_memories` toma `Json<ImportRequest>` con `text: String` (`crates/clawg-cli/src/daemon/api.rs:1869-1871`, `:1894-1897`), valida, y llama a `portability::import` o `import_foreign` con `.await` hasta el informe (`:1937-1950`). La ruta tiene un tope de cuerpo de `2 * MAX_IMPORT_BYTES` = 512 MiB «porque el escape JSON alarga el archivo» (`:1049`, `:1864-1866`). |
| El pico de memoria es varias veces el tamaño del archivo | **Cierta por construcción; la cifra no está medida.** Cliente: el `String` leído (1×), la copia que hace `json!` para el campo `text` (1×) y el `Vec<u8>` del cuerpo, más largo que el archivo por el escape (`"` pasa a `\"`; `client.rs:1325-1329`), de modo que el cliente tiene al menos tres copias a la vez. Daemon: el cuerpo en memoria, el `String` de `text` que deserializa serde (`api.rs:1871`) y, para Engram, un `serde_json::Value` del archivo entero (`crates/clawg-core/src/memory/portability/engram.rs:22-23`) antes de pasarlo al modelo de `foreign.rs`. El múltiplo exacto depende del archivo; la medición está en «Cómo se mide». |
| El tokenizador que se usa al traducir cuenta como uso del modelo y mantiene la inactividad a raya | **Falsa.** El tokenizador vive en su propia celda, `Arc<OnceCell<Arc<Tokenizer>>>` (`crates/clawg-core/src/runtime.rs:150`), que no se descarga nunca; `ChatChunker::from_cell` sólo lee esa celda (`api.rs:1940`). La marca de uso de `IdleEmbedder` sólo se toca en `lease`/`InFlight::drop` (`crates/clawg-core/src/memory/idle_embedder.rs:98-109`, `:112-141`), es decir, al llamar a `embed_*`. Traducir (~75 s) y la pasada de decisión (~800 s en la medición) no cuentan como uso. |
| Una importación puede sobrevivir a la ventana de inactividad (10 min por defecto) entre la traducción y el embedding | **Cierta.** `idle_after` da 10 minutos sin variable (`idle_embedder.rs:36`, `:46-57`); el barrido descarga el modelo cuando `in_flight == 0` y pasó el tiempo (`:61-63`, `:187-202`). `write` no toca el embedder hasta terminar la pasada de decisión (`import.rs:966-977` y luego `:982-992`), y esa pasada duró ~800 s: con el modelo ocioso desde antes, el barrido lo suelta en medio. Qué pasa entonces: **no es un error**, `embed_batch` pide un `lease` y recarga el modelo desde los archivos verificados (`idle_embedder.rs:128-139`, `:263-265`), segundos de espera una vez. Sólo si la recarga falla (`reload` devuelve error, o `RECOVERY_INTERVAL` frena una recuperación, `:119-126`) vuelve `EmbedError::NotReady`, que `import_memories` convierte en 503 `embedder_not_ready` (`api.rs:1408-1413`, `:1948`); como la pasada de decisión no se confirma (`import.rs:973`), no queda nada escrito, pero se pierden los ~800 s. La fase de escritura (~920 s) tampoco usa el modelo, y que se descargue ahí es deseable. |
| El daemon puede leer una ruta que el cliente nombra, y eso ya está resuelto de alguna forma | **Cierta, con una guarda mínima.** `POST /v1/projects` con `index_docs` acepta `root_path`, exige que sea un directorio (`api.rs:1991-1994`) y el daemon indexa sus archivos (`:2035-2037`). No hay lista de rutas permitidas: la guarda es que el daemon corre con el usuario, escucha en loopback o en un socket de su perfil, rechaza otro `Host`/`Origin` (`crates/clawg-cli/src/daemon/guard.rs`) y pide el token (`.claude/CLAUDE.md`, «Daemon HTTP API»); en Unix el socket descarta conexiones de otro uid (`admits_peer`). Quien tiene el token ya puede pedirle al daemon que lea cualquier ruta del usuario. |
| `/health` dice `ok` sin su bloque `ingest` mientras la pasada de decisión tiene la conexión | **Cierta.** `health` espera `outbox_summary` hasta `HEALTH_INGEST_WAIT` y, si vence, responde sin el bloque (`api.rs:1151-1169`); el estado no cambia. La consulta abandonada queda en la cola de la única conexión (read-connections, `openspec/changes/read-connections/exploration.md`, fila de `/health`). |
| Una importación cortada deja el store coherente | **Cierta.** Los lotes son de notas enteras (`note_batches`, `import.rs:938-952`) y cada uno se confirma solo (`:1008-1024`); la pasada de decisión no se confirma (`:973`). Una reimportación halla presentes las notas ya escritas y no cambia nada (`.claude/CLAUDE.md`, `clawg import`). |
| Un cliente que se va cancela la importación | **Sin verificar.** Hyper suele soltar el futuro del handler cuando el cliente cierra la conexión, lo que cortaría `write` en el siguiente `.await` entre lotes; pero una `conn.call` ya encolada no se cancela (`tokio-rusqlite` manda la clausura antes de esperar la respuesta, read-connections, tabla de premisas). Hoy ese podría ser el único «cancelar», sin informe. Un RED lo fija (ver «Cómo se mide»). |
| El número de una importación y el de un movimiento (`memory_move`) pueden compartirse | **Hoy no existe número de importación.** `move_id` es del registro de movimientos (V21, `crates/clawg-core/migrations/V21__memory_move.sql:11-12`); la importación escribe un snapshot `pre-import` y ningún registro propio (`import.rs:999`). 84 import-undo define ese número; ver «Decisiones abiertas». |

## Problema (verificado contra el código)

1. **Sin visibilidad.** La importación real tuvo cuatro fases (traducción ~75 s, decisión ~800 s, embeddings ~1.280 s, escritura ~920 s) y desde fuera sólo se veía un `clawg import` colgado; el estado se reconstruyó con un sondeo propio cada 20-50 s. El daemon conoce cada fase, cuántas filas planeó, cuántas embebió y cuántas escribió, pero no las expone (`write`, `import.rs:957-1038`).
2. **Sin cancelación.** No hay ruta para pedirlo. El cierre del cliente podría cortar la escritura entre lotes (premisa sin verificar), pero la pasada de decisión es una sola `conn.call` que no se interrumpe (`:966-977`) y no deja informe de qué quedó.
3. **Una petición de 51 minutos.** El cuerpo viaja entero, el cliente espera sin plazo (`client.rs:853`) y cualquier corte de la conexión (un proxy del sistema no entra, `client.rs`; sí un cierre del terminal, un reposo) pierde el informe aunque el daemon siga escribiendo.
4. **Memoria.** El archivo existe en varias copias en el cliente y en el daemon a la vez (premisa de pico), más el tope de cuerpo de 512 MiB (`api.rs:1866`) que sólo existe por el escape.
5. **El modelo se suelta entre fases.** Premisa de inactividad: una pasada de decisión larga deja que el barrido descargue el modelo y la fase de embedding paga una recarga, o falla con 503 y pierde la espera. Con 82 la ventana se acorta mucho; sigue valiendo para la traducción de un export enorme y para cualquier otra espera previa al primer `embed_batch`.
6. **El apagado.** Durante una importación, `HTTP_GRACE` (3 s) corta la petición (`crates/clawg-cli/src/daemon/serve.rs:77`, `:267-276`) y `runtime.shutdown` sólo conoce el ingest, los accesos y el mantenimiento (`runtime.rs:328-370`): no sabe que hay una importación. Si la conexión está dentro de la pasada de decisión, el cierre no corre (`serve.rs:283-288`: `shutdown` devuelve `false` cuando se pasa de su cota) y se espera lo que dure esa llamada.

## Lo que ya existe y se reutiliza

- `note_batches` (`import.rs:938`) da los límites de nota donde cancelar; `WRITE_BATCH` = 256 (`:55`) y `EMBED_BATCH` = 32 (`:57`) dan la granularidad del progreso y del chequeo de cancelación.
- `Ledger`/`ImportReport` ya cuentan lo que pasó; el informe final del trabajo es el mismo tipo (`report`, `import.rs:1040`) y `render` del cliente (`crates/clawg-cli/src/import.rs:76`) lo imprime sin cambios.
- `IdleEmbedder::wake` (`idle_embedder.rs:287-296`) y el contador `in_flight` (`:71-74`, `:98-109`) son la base del sostén del modelo.
- `GET /health` ya tiene un bloque opcional y un estado (`api.rs:559`, `:1151-1169`); un bloque `import` sacado de memoria del proceso no pide nada a la conexión.
- La guarda de `POST /v1/projects` (directorio, daemon del mismo usuario, token, `Host`/`Origin`) es el precedente de leer una ruta nombrada por el cliente (`api.rs:1991-1994`).
- El patrón de tareas en segundo plano del daemon: `spawn_doc_refresh` (`api.rs:2044`) y los workers de `Workers` (`runtime.rs:328-370`).
- `clawg import` ya valida y resuelve las raíces en el cliente antes de enviar (`crates/clawg-cli/src/import.rs:51-62`).
- `render`, `--dry-run`, `--agent`, `--map-root`, `--project-map` y la definición de `ImportOptions` no cambian.

## Enfoques posibles

### A. Trabajo asíncrono en el daemon, con el archivo por ruta (recomendado)

- `POST /v1/imports` con el cuerpo de hoy salvo que `text` se reemplaza por `path` (ruta absoluta, ya resuelta por el cliente). El daemon comprueba, antes de contestar, que sea un archivo regular, que no pase de `MAX_IMPORT_BYTES`, que sea legible, y contesta `202 {job_id}`; el resto de las validaciones de hoy (formato, `from`, mapas, tokenizador) pasan a la misma comprobación previa para que un error siga siendo un 400 inmediato y no un trabajo fallido. Un solo trabajo activo a la vez (un segundo `POST` es un 409 con el id del activo).
- `GET /v1/imports/{id}`: `phase` (`reading`, `translating`, `deciding`, `embedding`, `snapshot`, `writing`, `done`, `failed`, `cancelled`), `rows_planned`, `rows_embedded`, `rows_written`, `notes_done`, tasa, `started_at`, y el informe cuando termina. Sale de contadores atómicos que actualiza `write`; no toca la conexión.
- `DELETE /v1/imports/{id}`: levanta una bandera que `write` mira entre lotes de embedding y entre lotes de escritura (en el límite de nota que dan `note_batches`). Cancelar durante la decisión o el embedding no deja nada (el snapshot se toma después, `import.rs:999`); durante la escritura deja las notas ya confirmadas, que 84 deshace y una reimportación completa.
- `clawg import --follow`: `POST`, luego sondeo de `GET` cada 1-2 s mostrando fase y contadores, e imprime el informe de siempre al terminar; Ctrl-C pide `DELETE`. Sin `--follow` imprime el id y sale (el comportamiento síncrono de hoy queda como `--follow` por defecto en una terminal, a decidir).
- El trabajo sostiene el modelo (ver «Decisiones abiertas»).
- **A favor:** resuelve visibilidad, cancelación, plazo y memoria juntos con piezas que ya existen. **En contra:** una ruta y un registro de trabajos nuevos en el contrato del daemon; el daemon pasa a leer una ruta que otro le nombra (mismo nivel de confianza que `projects add`).

### B. Mantener el cuerpo, sólo hacerlo asíncrono

`POST /v1/imports` recibe `text` y devuelve 202; el resto igual. **A favor:** el cliente y el daemon pueden estar en máquinas distintas por un túnel (hoy no se soporta: el daemon es loopback). **En contra:** no resuelve la memoria ni el tope de 512 MiB; el cliente aún lee y escapa el archivo entero. Útil sólo como respaldo para un cliente más viejo.

### C. Respuesta en flujo en la misma petición

El `POST /v1/import` actual devuelve líneas de progreso (NDJSON) y al final el informe. **A favor:** sin registro de trabajos. **En contra:** cancelar es cerrar la conexión (la ambigüedad de la premisa sin verificar), no hay forma de volver a mirar un trabajo en curso desde otro terminal, y cualquier corte pierde el informe; el síntoma 3 no se resuelve.

### D. Dejar la petición como está y documentar `clawg export`/`projects` para trozos

Partir el archivo a mano. **En contra:** una nota partida entre dos archivos o un `topic_key` repartido se resuelven distinto; es trabajo del usuario donde el daemon ya conoce los límites de nota.

### Recomendación: A, con B como compatibilidad mientras haya clientes sin ruta

1. **Contrato.** `POST /v1/imports` acepta `path` o, para un cliente anterior, `text`; `POST /v1/import` (síncrono) sigue existiendo como un envoltorio: crea el trabajo y espera. Así no se rompe ningún cliente ni el CLI viejo.
2. **Estado en memoria del proceso**, sin tabla nueva: un trabajo vive mientras el daemon vive y se guarda el último terminado para consultar el informe. El registro durable de una importación (número, filas) es de 84.
3. **Cancelación y apagado** con una sola bandera: `DELETE` y el apagado la levantan; `write` para en el siguiente límite de nota y el trabajo queda `cancelled` o `interrupted`. El apagado espera al trabajo dentro de `AFTER_DRAIN` (7 s) porque el trabajo termina el lote en vuelo (milisegundos) y, desde 82, no hay una llamada de 800 s que esperar.
4. **`/health`** sigue con `status: ok` y gana un bloque `import` (fase y contadores, de memoria). Que `ok` siga diciéndose con la conexión ocupada es de read-connections (57, opciones C1-C3): este cambio no lo corrige, pero deja al menos `import.phase = deciding` visible para quien mira `/health`.
5. **Sostén del modelo** con un guardia (`hold`) en `IdleEmbedder` que suma a `in_flight` mientras vive; el trabajo lo toma desde el inicio de la fase de embedding (o desde la pasada de decisión si 82 la deja larga) y lo suelta al empezar la escritura, que no usa el modelo.

## Descartado

| Opción | Razón técnica |
|---|---|
| Subir `CLAWG_EMBEDDER_IDLE_MINUTES` o recomendar `0` durante una importación | Mantiene ~1 GB residentes todo el tiempo del daemon por una operación puntual; el sostén por trabajo lo limita a la fase que usa el modelo. |
| Que traducir cuente como uso (tocar `last_used` desde el tokenizador) | Acopla el tokenizador, que vive siempre (`runtime.rs:150`), a la vida del modelo; no cubre la pasada de decisión, que no usa ninguno de los dos. |
| Un plazo en el cliente (`.timeout`) como solución | Convierte una importación lenta en un error con el trabajo sin detener, y rompe la invariante de que sólo `health` tiene plazo (`client.rs:1784-1791`), que existe para no repetir una llamada que sí llegó al daemon. |
| Persistir el trabajo en `memory.db` para reanudarlo | Necesita una migración y una prueba que siembre la versión anterior, para un dato que una reimportación ya reconstruye (halla presentes las notas escritas y no cambia nada). |
| Reanudar un trabajo interrumpido desde su fase | Sin estado durable no hay desde dónde; la reimportación es idempotente y barata en las notas presentes. |
| Un servidor de archivos o subida por partes (`multipart`) | El cliente y el daemon son locales y del mismo usuario; una ruta evita la copia sin un protocolo nuevo. |
| Compartir el id del trabajo con `move_id` | `move_id` numera movimientos de filas (V21); un trabajo es un proceso volátil que puede no escribir nada (dry run, cancelado antes del snapshot). |

## Interacciones

- **import-linear (81) e import-decision-pass (82).** Cambian las fases: sin 82 la «decisión» es una llamada de ~800 s que no se puede cancelar ni mostrar; con 82 son lotes con puntos de control. El trabajo no depende de que 82 esté, pero cancelar «en la decisión» sólo es real con 82. Los contadores de progreso se definen sobre las fases de después de 82.
- **import-undo (84).** El número que 84 asigna a una importación (para `--undo N`) es persistente y se escribe al empezar a escribir; el id del trabajo es volátil y se entrega antes. El informe final del trabajo incluye el número de 84. Cancelar durante la escritura deja filas que 84 sabe deshacer: ese es el complemento del snapshot `pre-import`, no su reemplazo.
- **embed-throughput (85).** Cambia la velocidad del embedding y puede dar más granularidad (tamaño de lote, `EMBED_BATCH`, `embedder.rs:173`); los contadores de progreso usan el recuento de filas, no el tamaño de lote, para no depender de él. El sostén del modelo es el mismo que 85 necesita si cambia la forma de los lotes.
- **ranking-at-scale (86).** Sin relación directa; consume el store ya importado.
- **read-connections (57).** Es el dueño de que la lectura no espere a la escritura y de qué dice `/health` con la conexión ocupada (C1-C3). Con un lector, las lecturas de progreso nunca estarían en la cola de escritura; este cambio no las manda a la conexión, así que funciona antes y después. No duplica C1: el bloque `import` es el estado de un proceso, no un contador del outbox.
- **pipeline-metrics (73).** Los contadores del trabajo (filas por fase, tasa) son una fuente natural para las métricas de ingest; quedan como estructura legible por el escritor de minutos (`crates/clawg-cli/src/daemon/metrics.rs`) sin etiquetas que salgan del contenido. Registrarlos es de 73 si le sirven; este cambio no toca `metrics.db`.
- **shutdown-budget (55, archivado).** Las fases de apagado y su suma (`HTTP_GRACE`, `AFTER_DRAIN`, `RUNTIME_TEARDOWN`; `serve.rs:77-82`) están fijadas por una prueba de `stop.rs`; el trabajo debe caber en `AFTER_DRAIN` sin cambiar la suma. Un trabajo detenido por el apagado se informa `interrupted`.
- **history-import (44, archivado).** Define el formato y el informe que el trabajo reutiliza sin cambios.
- **Specs:** `openspec/specs/` del daemon HTTP y de la importación (rutas nuevas, trabajo, cancelación, ruta de archivo); y `.claude/CLAUDE.md` (sección `clawg import`) se actualiza al archivar.

## Alcance candidato

1. `POST /v1/imports` con `path` (y `text` por compatibilidad), validación previa idéntica a la de hoy, un trabajo a la vez, y `POST /v1/import` como envoltorio síncrono.
2. Registro de trabajos en memoria con fase y contadores atómicos actualizados por `write`; `GET /v1/imports/{id}`.
3. Cancelación por bandera en los límites de nota (`DELETE /v1/imports/{id}`) y por apagado, con estado final `cancelled`/`interrupted` e informe parcial.
4. `clawg import --follow` (y el id impreso sin él), con Ctrl-C que cancela.
5. Guardia de modelo (`hold`) en `IdleEmbedder`, tomado por el trabajo en la fase que usa el modelo.
6. Bloque `import` en `/health` sin consultar la conexión.
7. Specs y documentación de `clawg import`.

### No-alcance

- Persistir trabajos o reanudarlos (la reimportación es idempotente).
- Más de un trabajo a la vez.
- Mover el `status: ok` de `/health` con la conexión bloqueada (read-connections).
- El número durable de una importación y su deshacer (84).
- Acelerar el embedding (85), la pasada de decisión (82) o el escaneo de `PRESENT_SQL` (81).
- Registrar el progreso en `metrics.db` (73).
- Aceptar una ruta de otra máquina o de red.

## Decisiones abiertas

Son del usuario.

- [ ] **Lectura por ruta.** ¿El daemon lee la ruta que el cliente nombra (recomendado: quita las copias del archivo y el tope de 512 MiB; misma confianza que `projects add`), o se mantiene el cuerpo y sólo se hace asíncrono (B)? Si es por ruta: ¿se exige que sea absoluta y de un archivo regular dentro del perfil del usuario, o basta con lo que `projects add` exige hoy (que exista)?
- [ ] **Comportamiento por defecto de `clawg import`.** ¿Sigue esperando y mostrando progreso (`--follow` implícito en una terminal, recomendado) o imprime el id y sale, con `--follow` explícito? Un script que hoy espera el informe no debe romperse.
- [ ] **Número de trabajo y número de 84.** ¿El id del trabajo es volátil y distinto (recomendado) y el informe incluye el número durable de 84, o se unifican en un solo número persistido desde el `POST`? Unificar obliga a escribir una fila antes de saber si habrá algo que escribir (un dry run no escribe nada).
- [ ] **Sostén del modelo.** ¿Un guardia por trabajo (recomendado), `wake()` antes del embedding (más barato, sin garantía: la recarga puede fallar), o nada si 82 deja la ventana por debajo de los 10 minutos? Y si el modelo no carga al llegar a la fase, ¿el trabajo reintenta con espera o falla con `embedder_not_ready` como hoy?
- [ ] **Apagado con un trabajo activo.** ¿El apagado cancela el trabajo y espera el lote en vuelo dentro de `AFTER_DRAIN` (recomendado), o se rehúsa a apagar hasta que termine el trabajo? Lo segundo contradice las cotas de apagado.
- [ ] **Conservación del informe.** ¿Cuántos trabajos terminados recuerda el daemon (recomendado: el último), y por cuánto tiempo?
- [ ] **Un segundo `POST` con un trabajo activo.** 409 con el id del activo (recomendado) o una cola.

## Cómo se mide

**RED (cada uno falla hoy):**

1. `POST /v1/imports` contesta 202 sin esperar y `GET` muestra una fase y contadores que avanzan con un export de varios cientos de notas con el embedder falso (`CLAWG_TEST_EMBEDDER=fake`, `crates/clawg-cli/tests/binary.rs`). Hoy la ruta no existe (404).
2. `DELETE` durante el embedding deja el store sin una fila nueva y el trabajo `cancelled`; durante la escritura deja sólo notas enteras (ningún `memory_piece` sin cabeza) y el informe parcial las cuenta.
3. Con `path`, el cuerpo de la petición no contiene el archivo; una ruta que no existe, no es un archivo regular o supera `MAX_IMPORT_BYTES` es un 400 antes del 202.
4. `POST /v1/imports` con un trabajo activo es un 409 con su id.
5. Con tiempo pausado, un `hold` mantiene el modelo residente más allá de `IDLE` y, al soltarlo, la inactividad vuelve a correr (`idle_embedder.rs`); y una recarga que falla en la fase de embedding deja el trabajo `failed` con `embedder_not_ready` sin escribir nada.
6. `/health` con un trabajo activo informa `import.phase` y no encola ninguna consulta de la conexión (contador de llamadas en vuelo, como el RED 2 de read-connections).
7. Apagado: `POST /v1/shutdown` durante un trabajo lo deja `interrupted` y termina dentro de las cotas de `stop.rs`.
8. El cierre del cliente (premisa sin verificar): fija si hoy cancela, y que con el trabajo asíncrono ya no cancela por accidente.
9. El cliente: `--follow` imprime fases en orden y el informe idéntico al de `render`; Ctrl-C emite el `DELETE`. La prueba `only_health_sets_a_timeout` se mantiene verde.

**Medición:** pico de memoria (RSS máximo del cliente y del daemon) importando un archivo de tamaño fijo (por ejemplo 100 MiB generado) por cuerpo y por ruta, en el mismo build; el múltiplo del archivo en cada caso. Y la frecuencia y el costo de las lecturas de progreso con el embedder falso. No cambia ranking: no se corre `rag-eval`. No hay migración.

## Riesgos

- **Superficie nueva en el daemon.** Una ruta de archivo que otro nombra. La guarda es la de `projects add` (token, `Host`/`Origin`, mismo usuario, peer en Unix); aun así los motivos de las líneas inválidas del informe no deben citar contenido de un archivo que no es un export (hoy citan el motivo del parser, `import.rs` del cliente `:148-157`).
- **Dos fuentes de estado del progreso.** El informe final y los contadores deben coincidir; una prueba compara los totales.
- **Cancelar a medias deja un store parcial.** Es coherente por notas enteras, pero el usuario debe saberlo: el informe lo dice y apunta a `restore` y a 84.
- **Un trabajo que sobrevive al cliente.** Si nadie lo mira ni lo cancela, escribe hasta el final. Es lo que se quiere, pero hay que decirlo en la salida de `clawg import` sin `--follow`.
- **Orden con 82.** Si el trabajo llega antes que 82, su cancelación no actúa durante la pasada de decisión; debe documentarlo hasta entonces.
- **El sostén del modelo retiene ~1 GB.** Sólo en la fase que lo usa; una prueba fija que se suelta al empezar la escritura.
