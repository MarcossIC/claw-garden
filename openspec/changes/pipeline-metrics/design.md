# pipeline-metrics — Diseño

## D1. La sonda mide la cola, no la vuelta (P1)

**Elegido:** `MemoryStore::probe_wait` toma un `Instant` antes de encolar y otro como primera instrucción del cierre que corre en el hilo de la conexión; la espera es la diferencia. El cierre no ejecuta SQL: lo que una consulta paga detrás de otra es la cola, y un `SELECT 1` sólo sumaría su propio tiempo y la vuelta por el canal.

La sonda corre en una tarea del daemon (`daemon/store_probe.rs`) con un `tokio::time::interval` de `PROBE_INTERVAL` (10 s) y `MissedTickBehavior::Delay`: una sonda que esperó 30 s detrás de un `reembed_all` no deja tres sondas en ráfaga detrás de sí. El primer tic es inmediato, así que un daemon recién arrancado ya tiene una muestra. La tarea se aborta cuando los servidores paran y antes de que el runtime drene y cierre el store, así ninguna sonda queda en la cola del cierre.

| Alternativa | Por qué no |
|---|---|
| `call_timed` en cada llamada | Toca cada llamada del store; la sonda da la misma espera con un cambio local. Si un día hace falta el desglose por llamada, se migra sólo el camino caliente |
| Leer la profundidad de la cola | `tokio_rusqlite` no la expone; la espera es lo que pide la puerta de 57 |
| Sonda cada segundo | Diez veces más llamadas para una resolución que el minuto agrega igual; 10 s ya ve una operación larga |

## D2. La lectura por minuto (P2, P3, P4)

**Elegido:** la misma tarea, en el primer tic y luego cada `MINUTE_EVERY` (6) tics, hace además la lectura por minuto, en este orden: `MemoryStore::cache_status` (una llamada en la conexión que lee `sqlite3_db_status` con `resetFlg = 1` para los contadores, así cada lectura da lo de su minuto), el tamaño de los ficheros en un `spawn_blocking` (son `metadata` de unos pocos ficheros, pero bloquean) y `outbox_summary`, la misma consulta que contesta `/health`.

`sqlite3_db_status` no tiene envoltorio seguro en `rusqlite` 0.37: se llama por `rusqlite::ffi` con `Connection::handle`, como ya hace `sqlite3_limit` en los tests de `db.rs`. Las dos lecturas viven en `memory/conn_probe.rs`, un `impl MemoryStore` aparte como el de `outbox.rs`. Es una lectura de contadores de la propia conexión, dentro de su hilo.

Los gauges caen en el minuto del `Recorder` que el escritor de 71 toma en el cambio del minuto del reloj; la lectura no está alineada con ese cambio, y no hace falta: un gauge es el valor que había, y uno por minuto alcanza para ver la tendencia.

Un fallo de cualquiera de las tres partes deja fuera sólo sus series, con un `debug`: el store cerrado o un fichero que desapareció no son noticia.

## D3. El `Recorder` en el runtime (P5 a P8)

**Elegido:** `MemoryRuntime` lleva un `Arc<Recorder>` y se lo pasa al worker de ingest, a la búsqueda y al embedder; `serve` crea el runtime con el mismo que pone en `AppCtx`. Un runtime de test crea uno propio, así los tests en proceso miden igual que el daemon. El `Recorder` ya vive en `clawg-core::metrics`, así que no hay dependencia nueva.

| Alternativa | Por qué no |
|---|---|
| Un `Recorder` global (`OnceLock`) | Dos runtimes en un proceso de test mezclarían sus series |
| Devolver tiempos y que el daemon los registre | Cada función devolvería un informe que sólo el daemon lee; el ingest corre en su worker, fuera de toda petición |

## D4. `ingest_pieces` partida antes de medirla (P6)

**Elegido:** partirla en funciones por etapa (preparar las piezas, embeberlas, guardarlas, enlazarlas) en commits propios, cada uno con sus tests, sin cambiar lo que guarda; sólo después se envuelve cada etapa con su `Instant`. Medir dentro de la función tal como está pondría seis `Instant` en 240 líneas y dejaría las etapas sin test.

## D5. Eventos en `metrics.db` (P9)

**Elegido:** una tabla `event` en `metrics.db`, `user_version = 2`, con `kind` de un conjunto cerrado y `detail` JSON de números y valores cerrados. Se escribe con el minuto, desde una cola del `Recorder`, así los eventos no abren otra escritura. La migración es `CREATE TABLE IF NOT EXISTS` y subir `user_version`: aditiva, y un binario anterior rechaza el fichero sin tocarlo, como ya hace con un esquema más nuevo.
