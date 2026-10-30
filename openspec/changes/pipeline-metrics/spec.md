# pipeline-metrics — Especificación

Delta sobre [`specs/metrics`](../../specs/metrics/spec.md). Las series se registran en el `Recorder` de R2 de esa spec, se guardan por minuto en `metrics.db` (R3, R4) y se leen por `GET /v1/metrics` (R7) y `clawg metrics report` (R8) sin cambio en ninguno de ellos, salvo P9.

Toda etiqueta de este cambio MUST venir de un conjunto cerrado escrito en el código; ninguna MUST venir del texto de una memoria, de una consulta, de una ruta de fichero, de un id o de un `agent_id`. Ninguna serie cambia lo que el daemon hace ni lo que contesta.

## P1. La espera en la conexión del store

El daemon MUST encolar en la conexión de `memory.db`, al arrancar y luego cada `PROBE_INTERVAL` (10 s), una llamada sin SQL, y MUST registrar como `db.probe_wait` (`us`) el tiempo desde que la encola hasta que empieza a correr en el hilo de la conexión. Una sonda que no corre (la conexión cerrada) MUST NOT registrar nada. La sonda MUST parar cuando los servidores paran, antes de que el store se cierre.

#### Escenario: una sonda detrás de una llamada larga
- GIVEN una llamada que ocupa la conexión 200 ms
- WHEN una sonda se encola detrás
- THEN la sonda mide una espera de al menos 150 ms

**Tests:** `the_probe_waits_behind_a_call_in_progress`, `a_closed_connection_is_an_error_not_a_reading` (`crates/clawg-core/src/memory/conn_probe.rs`); `a_probe_records_its_wait` (`crates/clawg-cli/src/daemon/store_probe.rs`).

## P2. La caché de SQLite

Una vez por minuto (la primera sonda y luego cada sexta), dentro de una llamada en la conexión, el daemon MUST leer `sqlite3_db_status` y registrar como gauges `db.cache_hit`, `db.cache_miss`, `db.cache_write` y `db.cache_spill` (lo contado desde la lectura anterior: cada lectura los pone a cero) y `db.cache_used_bytes` y `db.stmt_used_bytes` (lo que hay ahora). Un contador que SQLite no da MUST faltar.

#### Escenario: los contadores empiezan de cero en cada lectura
- GIVEN una lectura del estado y luego unas consultas
- WHEN se lee dos veces seguidas
- THEN la primera trae los aciertos de esas consultas y la segunda ninguno

**Tests:** `the_cache_counters_restart_at_each_reading` (`conn_probe.rs`).

## P3. El tamaño de los ficheros del store

En la misma lectura por minuto, fuera de los hilos del runtime, el daemon MUST registrar `db.file_bytes` con `file` en `db` (`memory.db`), `wal` (`memory.db-wal`) y `backups` (la suma de los ficheros de `<data>/backups`, sin bajar a sus subdirectorios). Un fichero o directorio que no existe MUST dejar su serie fuera, nunca en 0.

**Tests:** `the_store_files_are_measured_and_a_missing_one_is_left_out` (`crates/clawg-cli/src/daemon/store_probe.rs`).

## P4. El outbox en el tiempo

En la misma lectura por minuto, el daemon MUST registrar los gauges `outbox.pending` y `outbox.failed` y, con una fila pendiente, `outbox.oldest_pending_s`, de `OutboxSummary` (`crates/clawg-core/src/memory/outbox.rs:147`). Sin filas pendientes, `outbox.oldest_pending_s` MUST faltar.

**Tests:** `a_minute_reading_records_the_cache_the_files_and_the_outbox` (`store_probe.rs`); `a_daemon_probes_its_store_without_being_asked` (`crates/clawg-cli/tests/binary.rs`).

## P5. Búsqueda y contexto

Cada búsqueda de memorias MUST registrar `search.duration` (`us`) con `arm` en `knn_memory`, `knn_episodic`, `knn_skill`, `lexical`, `identifiers` y `total`; `search.knn_retry` con `table` cuando el KNN de esa tabla se repitió (`KnnOutcome::retried`, `crates/clawg-core/src/memory/search.rs:446`); `search.hits` y `search.withheld` como gauges. Cada `POST /v1/context` MUST registrar `context.retrieval_state` con `state` (`searched`, `recent`, `embedder_not_ready`, `embedder_failed`, `search_failed`), `context.prompt_bytes` (`bytes`) y, por bloque recortado por el presupuesto, `context.cut` con `block` (`recent`, `long_term`, `last_session`, `conflicts`).

**Tests:** `a_search_records_how_long_each_arm_took`, `a_widened_window_is_counted_by_its_table` (`crates/clawg-core/src/memory/search.rs`); `the_fit_names_each_block_it_cut` (`prompt_builder.rs`); `a_search_records_its_arms_its_hits_and_what_it_withheld`, `a_context_records_its_state_its_size_and_the_blocks_it_cut` (`crates/clawg-cli/tests/api.rs`).

## P6. Ingest por etapa

`ingest_pieces` (`crates/clawg-core/src/memory/worker.rs:754`) MUST partirse en una función por etapa, sin cambiar lo que guarda, con un test por etapa. Cada save MUST registrar `ingest.stage` (`us`) con `stage` por cada etapa que corrió, `ingest.pieces` (las piezas que guardó), `ingest.end_to_end` (`us`, desde el `created_at` de su fila del outbox hasta que termina) y `ingest.outcome` con `outcome` (`stored`, `retrying`, `partially_saved`, `failed`). El worker MUST registrar la profundidad de la cola, `ingest.queue_depth`, cada vez que toma un save.

**Tests:** `the_split_masks_the_text_and_leaves_out_what_stores_nothing`, `the_gate_absorbs_a_stored_piece_and_takes_a_resumed_one_as_it_was`, `the_embed_puts_the_head_before_every_piece_but_the_first_and_names_its_failure`, `the_store_writes_each_piece_notes_it_and_pauses_when_a_drain_is_due`, `a_take_is_named_by_how_it_ended` (`crates/clawg-core/src/memory/worker.rs`); `an_accepted_save_records_its_stages_its_pieces_and_how_long_it_took` (`crates/clawg-core/src/runtime.rs`).

## P7. Embedder

Cada lote MUST registrar `embedder.batch` (textos) y `embedder.batch_tokens` (los tokens con el relleno) con `role` (`query`, `passage`), y `embedder.wait` (`us`, la espera hasta que el hilo de inferencia toma el lote) con `role`. Cada carga MUST registrar `embedder.load` (`us`) con `phase` (`verify`, `session`).

**Tests:** `every_batch_records_its_texts_by_role_and_the_worker_its_wait` (`crates/clawg-core/src/memory/embedder.rs`); `a_load_records_how_long_it_verified_and_built_and_a_failure_records_nothing` (`bootstrap.rs`). `embedder.batch_tokens` sale de la sesión del modelo real, que la suite no carga.

## P8. Docs

Cada refresco del índice de docs MUST registrar `docs.refresh.duration` (`us`) y `docs.refresh.files` con `state` (`indexed`, `unchanged`, `removed`, `failed`); cada `POST /v1/projects/docs/search` que espera un refresco, `docs.search.inline_wait` (`us`).

**Tests:** `a_docs_search_records_the_refresh_and_how_long_it_waited_for_it` (`crates/clawg-cli/tests/api.rs`).

## P9. Eventos

`metrics.db` MUST subir a `user_version = 2` con una tabla `event` (`run_id`, `at_ms`, `kind`, `detail` JSON de números y valores de conjuntos cerrados), aditiva sobre el esquema 1, que un fichero del esquema 1 recibe al abrirse sin perder filas. MUST registrarse un evento por carga y descarga del modelo (con su causa y duración), recuperación del embedder, descarga del modelo (resultado y bytes), pasada de mantenimiento (duración y recuentos), backup, import, fila del outbox que pasa a `failed` y apagado (la duración de cada fase). Los eventos MUST podarse con las horas (R4 de `specs/metrics`), y `clawg metrics report --events [--since]` MUST listarlos por ejecución.

**Tests:** `a_file_of_schema_1_gets_its_events_and_keeps_its_rows`, `events_past_the_retention_go_with_their_run` (`crates/clawg-core/src/metrics/store.rs`); `events_wait_for_the_writer_and_the_run_keeps_the_latest` (`recorder.rs`); `the_events_of_a_minute_are_kept_with_it_and_served_with_the_run` (`crates/clawg-cli/src/daemon/metrics.rs`); por tipo, `each_load_unload_and_recovery_is_an_event` (`idle_embedder.rs`), `a_download_is_an_event_and_a_verified_cache_is_none` (`bootstrap.rs`), `a_maintenance_pass_is_an_event_with_what_it_did`, `a_snapshot_is_an_event_with_its_reason_and_size`, `a_row_that_turns_failed_is_an_event_with_its_cause`, `an_import_and_its_snapshot_are_events_of_the_run`, `a_shutdown_event_names_its_phases_in_milliseconds`; `the_events_are_listed_by_run` (`crates/clawg-cli/src/metrics.rs`).

## P10. De punta a punta

Un test de `binary.rs` MUST guardar con el embedder falso un texto de varias piezas, buscar, refrescar un índice de docs pequeño y encontrar en `GET /v1/metrics` las series de P1 a P8 y, tras `clawg stop`, los eventos de P9 en `metrics.db`. El modelo falso no tiene sesión que mida lotes, así que `embedder.batch`, `embedder.batch_tokens` y `embedder.wait` quedan para los tests del embedder.

**Tests:** `a_daemon_measures_its_pipeline_and_keeps_its_events` (`crates/clawg-cli/tests/binary.rs`).

## Invariantes

- `memory.db`, su esquema y lo que ve el modelo no cambian.
- Ninguna etiqueta ni fila lleva texto de una memoria, una consulta, una ruta de fichero o un id.
- Una medición que falla se deja fuera; nunca falla una petición ni para un worker.
