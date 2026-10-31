# pipeline-metrics — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal | M | 71 (usage-metrics, archivado) | ver [`state.yaml`](state.yaml) |

## Problema

Detalle y evidencia en la [exploración](exploration.md); verificado otra vez contra `develop`:

1. **No se ve la espera en la única conexión del store.** Toda lectura y escritura de `memory.db` pasa por un `tokio_rusqlite::Connection` (`MemoryStore::conn`, `crates/clawg-core/src/memory/db.rs:149`), un hilo con una cola. Lo que espera detrás de un refresco del índice de docs, de un save de muchas piezas o de un `reembed_all` (`crates/clawg-core/src/memory/maintenance.rs:570`) sólo se ve como parte de `http.duration`, mezclado con el resto de la petición. Es la puerta de read-connections (57).
2. **No se ve el estado del store.** Ni la caché de páginas de SQLite, ni el tamaño de `memory.db`, de su `-wal` (acotado a 64 MiB tras un checkpoint, `WAL_SIZE_LIMIT`, `db.rs:179`) o de `backups/`, ni el outbox en el tiempo: `OutboxSummary` (`crates/clawg-core/src/memory/outbox.rs:147`) sólo sale como foto en `/health`.
3. **No hay series por etapa.** La búsqueda (`search_all_memories_inner`, `crates/clawg-core/src/memory/search.rs:1248`, y su reintento del KNN, `KnnOutcome::retried`, `:446`), el ingest (`ingest_pieces`, `crates/clawg-core/src/memory/worker.rs:754`, y la profundidad de la cola, que sólo avisa al pasar 64, `:106`, `:339`), los lotes del embedder (`EMBED_BATCH`, `crates/clawg-core/src/memory/embedder.rs:173`) y el refresco del índice de docs (`search_docs`, `crates/clawg-cli/src/daemon/api.rs:2760`) no dejan ninguna serie. Una regresión de `POST /v1/context` medida por 71 no dice si fue la búsqueda, la espera en SQLite o el embedder.
4. **Los eventos raros de cada ejecución viven sólo en el log.** Una carga del modelo (`crates/clawg-core/src/memory/idle_embedder.rs:196`), una recuperación (`RECOVERY_INTERVAL`, `:42`), una pasada de mantenimiento o una fila que pasa a `failed` no se pueden contar por versión.

## Objetivo que sirve

Transversal: dar datos de uso real a las puertas que esperan una medición (57, 62, el crowding de 37) con una sola instrumentación sobre el `Recorder` y `metrics.db` de 71, en lugar de un banco por puerta.

## Decisiones

Las decisiones abiertas de la exploración se cierran así; el porqué está en el [diseño](design.md).

- **La sonda corre cada 10 s** y mide la espera en la cola de la conexión hasta que su llamada empieza a correr, no la vuelta entera: lo que una consulta paga detrás de otra es la cola.
- **`ingest_pieces` se parte por etapas** en commits propios, con tests por etapa, antes de instrumentarla.
- **El `Recorder` pasa al runtime** (`MemoryRuntime`) para lo que mide `clawg-core`; el daemon le da el mismo a `AppCtx`. Un runtime de test lleva uno propio.
- **Los eventos van a `metrics.db`**, en una tabla `event` que sube su esquema a `user_version = 2`; 71 dejó fuera la tabla a propósito para este cambio.

## Alcance

Por tramos, cada uno un grupo de commits que se sostiene solo:

1. **SQLite y outbox** (M4): `db.probe_wait`, el estado de la caché de SQLite (`sqlite3_db_status`) una vez por minuto, el tamaño de `memory.db`, `memory.db-wal` y `backups/`, y los gauges del outbox (`outbox.pending`, `outbox.failed`, `outbox.oldest_pending_s`).
2. **Búsqueda y contexto** (M5): `search.duration{arm}`, `search.knn_retry{table}`, `search.hits`, `search.withheld`, `context.retrieval_state{state}`, `context.prompt_bytes`, `context.cut{block}`.
3. **Ingest** (M5): `ingest_pieces` partida; `ingest.end_to_end`, `ingest.queue_depth`, `ingest.pieces`, `ingest.stage{stage}`, `ingest.outcome{outcome}`.
4. **Embedder** (M5): `embedder.batch{role}`, `embedder.batch_tokens`, `embedder.wait{role}`, `embedder.load{phase}`.
5. **Docs** (M5): `docs.refresh.duration`, `docs.refresh.files{state}`, `docs.search.inline_wait`.
6. **Eventos** (M6): la tabla `event` y sus tipos cerrados, `GET /v1/metrics` y `clawg metrics report --events`.
7. **De punta a punta** y docs.

## Fuera de alcance

- Cambiar comportamiento: este cambio sólo observa. Un lector aparte (57), menos hilos (62) o un umbral de aviso son de su cambio.
- `call_timed` en cada llamada a la conexión: la sonda da la espera con un cambio local.
- La instantánea diaria del dominio (M7) y el banco por versión (74).
- Lo que ve el modelo: `agent_contract.json` no cambia.

## Cómo se mide el éxito

Tests que fallan hoy (ver [spec](spec.md)), sin modelo real, red ni perfil:

- con la conexión ocupada por una llamada de 200 ms, la sonda que se encola detrás mide al menos esa espera;
- los contadores de la caché empiezan de cero en cada lectura;
- un daemon con `--port 0` y el embedder falso trae `db.probe_wait`, los gauges de la caché, de los ficheros y del outbox en `GET /v1/metrics` sin que nadie los pida;
- tras un save de varias piezas, una búsqueda y un refresco de docs, cada serie de los tramos 2 a 5 está, y cada evento del tramo 6 queda en `metrics.db`;
- ninguna etiqueta viene de un texto, una consulta, una ruta o un id.

El sobrecoste se mide como en 71: un `SELECT`-sin-SQL cada 10 s en la cola de la conexión es despreciable frente a lo que hace una petición; el umbral, p95 de `http.duration{route=/v1/search}` sin diferencia distinguible con y sin la sonda, en el perfil dev con el modelo real cuando haya autorización.

## Datos y vuelta atrás

`memory.db` no cambia: ni esquema, ni identificadores, ni estados, ni contrato de embedding. `metrics.db` sube a `user_version = 2` con una tabla nueva (`event`), aditiva. Un binario anterior rechaza un esquema más nuevo sin tocarlo y no escribe métricas hasta el próximo arranque (R3 de [`specs/metrics`](../../../specs/metrics/spec.md)); sirve igual. Volver atrás es volver al binario anterior y, para que vuelva a escribir métricas, borrar `metrics.db`, que sólo pierde su historia. Los tramos 1 a 5 no tocan el esquema.
