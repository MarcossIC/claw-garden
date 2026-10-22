# pipeline-metrics — Exploración

Objetivo transversal. Sobre la base de [usage-metrics](../usage-metrics/exploration.md) (71), mide dónde se va el tiempo dentro del daemon: la espera en la única conexión de SQLite, las ramas de la búsqueda, las etapas del ingest, el embedder y el índice de docs, más los eventos raros de cada ejecución. Junta las propuestas M4, M5 y M6 de una propuesta de medición escrita sobre 0.26.0. Su razón de ser hoy es dar datos a las puertas de medición que ya esperan en read-connections (57), embedder-footprint (62) y docs-refresh-background (64), en lugar de que cada una arme su propia medición.

## Premisas que se verificaron y cuáles no eran del todo ciertas

| Premisa o métrica propuesta (sobre 0.26.0) | Estado en `develop` (0.45.0) |
|---|---|
| Una operación larga bloquea la única conexión; el `VACUUM INTO` de un backup hizo esperar 631 ms a una consulta de 0,05 ms | **Cierta en lo general, ya no para el backup.** El backup corre en su propia conexión desde `b1f22f3` (`snapshot_into`, `crates/clawg-core/src/memory/backup.rs:336`). Lo que queda en la cola son los refrescos del índice de docs, los saves de muchas piezas y las ráfagas: es el problema de read-connections (57), cuya puerta (`openspec/changes/read-connections/exploration.md`, «Cómo se mide») pide la espera de un save, de `session-start` y de una búsqueda durante una operación larga, y la profundidad de la cola. `db.probe_wait` es esa serie en producción. |
| `db.op_duration{op=snapshot}` | **Pierde peso**: el snapshot ya no frena a nadie. Quedan la pasada de mantenimiento, `reembed_all` (`crates/clawg-core/src/memory/maintenance.rs:507`) e `import`. |
| `ingest.chunker_mode`: el chunker queda en modo bytes toda la vida del proceso | **Obsoleta.** Arreglado en `adab212`: `ChatChunker::from_cell` lee la celda del tokenizer en cada save (`openspec/README.md`, «Ideas evaluadas»). Se descarta. |
| `ingest.progress_bytes` para ver el crecimiento cuadrático | **Pierde peso.** save-bounds (54) puso un tope de 64 KiB a `content`, que acota el progreso (8,3 MB a 64 KiB, medido). `ingest.pieces` sigue siendo útil para ver la distribución real. |
| `outbox.oldest_pending_s` para ver una fila `pending` sin nadie esperando | **Barata:** `OutboxSummary` ya la calcula (`crates/clawg-core/src/memory/outbox.rs:147-154`) y `/health` la expone. outbox-lifecycle (53) cambió cuándo una fila queda `pending` o `failed`; la serie sirve para verlo en uso real. |
| La cola de ingest sólo avisa al pasar 64 | **Cierta.** `INGEST_QUEUE_WARN_DEPTH` (`crates/clawg-core/src/memory/worker.rs:105`) y el cruce en `:133`; la profundidad ya se lee en `:336`. |
| Las etapas de `ingest_pieces` están comentadas | **Cierta.** `ingest_pieces` (`worker.rs:719`) sigue siendo una función de unas 240 líneas; el README lista partirla por etapas como arreglo para cuando un cambio la toque, y este la toca. |
| `search.knn_retry{table}` desde `KnnOutcome::retried()` | **Disponible.** `retried` está en `crates/clawg-core/src/memory/search.rs:426` y `search_all_memories_inner` en `:1196`. El reintento es el síntoma que knn-prefilter (37) quiere medir con un corpus multiagente; en uso real es la misma pregunta con datos reales. |
| Descarga sin timeout y causa perdida (`model.download{outcome}`) | **Arreglado por model-download (60):** 30 s para conectar, 60 s entre lecturas, causa con host. Contar descargas y su duración sigue siendo útil como evento. |
| Leer y hashear el modelo en el runtime async (0,3 a 2 s estimados) | **Movido a `spawn_blocking` por 60.** El tiempo sigue sin serie: `bootstrap.rs:452-467` ya separa lectura y sesión en el log, e `idle_embedder.rs:127-137` registra la recarga en ms. Pasarlo a un histograma es trivial. Medido al archivar 60: con la caché dañada, una recarga tarda 8,7 a 9,2 s en debug en Windows antes de decir «no listo» (backlog). |
| Un pánico del embedder deja `WorkerGone` para siempre | **Arreglado por embedder-recovery (61):** recarga y repite una vez, como mucho una vez por minuto. Contar recuperaciones como evento dice si pasa en uso real, cosa que 61 no pudo encontrar. |
| Una consulta espera detrás de lotes de docs o de un re-embed | **Acotado:** el canal manda un lote de 16 y espera (`crates/clawg-core/src/memory/embedder.rs:418`), así que hay una petición por llamante concurrente (README, «Ideas evaluadas»). `embedder.wait{role}` sigue siendo la medida que embedder-footprint (62) necesita para decidir hilos y lotes. |
| `search_project` refresca en línea sin cota | **Cierta, y es el cambio 64.** `search_docs` llama a `try_refresh` antes de buscar (`crates/clawg-cli/src/daemon/api.rs:1801-1819`). `docs.search.inline_wait` y `docs.refresh.duration` son la medida de antes y después de 64. Hoy el refresco no registra ni su duración. |
| El presupuesto corta LONG-TERM antes que RECENT | **Arreglado** en `570488d`. `context.cut{block}` queda como dato de uso, no como hallazgo. |
| LAST SESSION y RECENT recorren la tabla | **Arreglado** por query-plans (56). `http.duration{route=/v1/context}` contra el tamaño del store lo vigila. |

## Problema

Las cuatro puertas abiertas que dependen de medir (57, 62, 64 y el crowding de 37) proponen cada una un banco o una prueba propia, y ninguna tiene datos de uso real. Sin series por etapa, una regresión de `POST /v1/context` medida por 71 no dice si fue la búsqueda, la espera en SQLite o el embedder.

## Alcance candidato

1. **SQLite** (M4): una tarea que cada 10 s encola un `SELECT 1` en la conexión del store y mide la vuelta (`db.probe_wait`); `sqlite3_db_status` (`CACHE_HIT`, `CACHE_MISS`, `CACHE_WRITE`, `CACHE_SPILL`, `CACHE_USED`, `STMT_USED`) una vez por minuto dentro de un `call`, por `rusqlite::ffi` (libsqlite3-sys 0.35.0 expone `sqlite3_db_status`; un `unsafe` de una línea); tamaño de `memory.db`, `memory.db-wal` y `backups/`; `db.op_duration{op}` alrededor de mantenimiento, `reembed_all`, `import` y `purge`.
2. **Búsqueda y contexto** (M5): `search.duration{arm}` (KNN por tabla, léxico, identificadores, total), `search.knn_retry{table}`, `search.hits`, `search.withheld`, `context.retrieval_state{state}`, `context.prompt_bytes`, `context.cut{block}`, `search.no_relevant` y las filas vivas del store como gauge para normalizar.
3. **Ingest** (M5): `ingest.end_to_end` (de `created_at` de la fila del outbox a su fin), `ingest.queue_depth` (máximo del minuto), `ingest.pieces`, `ingest.stage{stage}` con `ingest_pieces` partida por etapa, `ingest.outcome{outcome}` y los gauges del outbox.
4. **Embedder** (M5): `embedder.batch{role}` y `embedder.batch_tokens` (útiles contra padding), `embedder.wait{role}`, `embedder.load{phase}`.
5. **Docs** (M5): `docs.refresh.duration`, `docs.refresh.files{state}`, `docs.search.inline_wait`.
6. **Eventos** (M6), en la tabla `event` de 71: carga y descarga del modelo, recuperación de 61, descarga del modelo con su resultado y bytes, pasada de mantenimiento con su duración y recuentos, backup, import, apagado con la duración de cada fase de 55, y una fila del outbox que pasa a `failed`.

### No-alcance

- Cambiar comportamiento: este cambio sólo observa. Si una puerta decide algo (un lector aparte, menos hilos, refresco en segundo plano), es su cambio.
- `call_timed` en cada una de las llamadas a la conexión: la sonda periódica da la espera con un cambio local; migrar sólo el camino caliente si hace falta el desglose.
- La instantánea diaria del dominio (M7): ver «Ideas evaluadas» del README.

## Decisiones abiertas

- [ ] **Partir `ingest_pieces`.** Hacerlo aquí (lo pide la instrumentación de etapas) o medir con `Instant` dentro de la función tal como está. Recomendación: partirla, con tests por etapa, en commits propios antes de instrumentar.
- [ ] **Intervalo de la sonda.** 10 s es un `SELECT 1` en la cola cada 10 s; insignificante, pero se mide con el sobrecoste de 71.

## Interacciones

- **read-connections (57).** `db.probe_wait` durante un refresco o un save largo es la medida de su puerta en producción; su prueba en `binary.rs` puede leer `GET /v1/metrics` en vez de muestrear `/health`.
- **embedder-footprint (62).** `embedder.batch`, `embedder.batch_tokens`, `embedder.wait` y la CPU de 71 son su tabla de puerta en uso real; el banco con el modelo real sigue siendo suyo (o de 74).
- **docs-refresh-background (64).** Medir `docs.search.inline_wait` antes de 64 da la línea de base que 64 tiene que bajar. Si 64 se hace antes, sólo mide el después.
- **knn-prefilter (37).** `search.knn_retry` en uso real dice si el crowding aparece fuera de su corpus.

## Cómo se mide

Tests unitarios de cada etapa partida; un test en `binary.rs` que, con el embedder falso, guarda un texto de varias piezas, busca, refresca un índice de docs pequeño y comprueba que las series y los eventos esperados están en `GET /v1/metrics`; el sobrecoste, como en 71.
