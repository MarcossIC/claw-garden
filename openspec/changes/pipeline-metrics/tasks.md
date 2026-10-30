# pipeline-metrics — Tareas

Strict TDD: en cada tarea con conducta nueva el test va primero y falla por la aserción o por no compilar. Una tarea, un commit convencional.

## Tramo 1: SQLite y outbox

### 1. La espera y la caché de la conexión (P1, P2, D1, D2)

- [x] Tests primero: `the_probe_waits_behind_a_call_in_progress`, `the_cache_counters_restart_at_each_reading`, `a_closed_connection_is_an_error_not_a_reading` (`crates/clawg-core/src/memory/conn_probe.rs`).
- [x] `MemoryStore::probe_wait` y `MemoryStore::cache_status` (`CacheStatus`).
- Commit: `feat(core): measure the wait on the store connection and its page cache`

### 2. La sonda del daemon (P1 a P4, D1, D2)

- [x] Tests primero: `the_store_files_are_measured_and_a_missing_one_is_left_out`, `a_minute_reading_records_the_cache_the_files_and_the_outbox`, `a_probe_records_its_wait` (`crates/clawg-cli/src/daemon/store_probe.rs`).
- [x] `daemon/store_probe.rs`; `serve` la arranca y la aborta cuando paran los servidores.
- Commit: `feat(daemon): probe the store connection, its cache, its files and the outbox`

### 3. De punta a punta del tramo (P4)

- [x] `a_daemon_probes_its_store_without_being_asked` en `crates/clawg-cli/tests/binary.rs`.
- Commit: `test(cli): check that a real daemon probes its store`

### 4. Docs del tramo

- [x] `.claude/CLAUDE.md`: las series de la sonda.
- Commit: `docs: describe the store probe metrics`

## Tramo 2: búsqueda y contexto (P5, D3)

- [ ] El `Recorder` en `MemoryRuntime` y el mismo en `AppCtx`; un test de que un runtime de test lleva el suyo.
- [ ] Tests primero de `search.duration{arm}`, `search.knn_retry{table}`, `search.hits`, `search.withheld`; luego la búsqueda.
- [ ] Tests primero de `context.retrieval_state`, `context.prompt_bytes`, `context.cut{block}`; luego el contexto.

## Tramo 3: ingest (P6, D4)

- [ ] Partir `ingest_pieces` por etapa, un commit por etapa con sus tests, sin cambiar lo que guarda.
- [ ] Tests primero de `ingest.stage`, `ingest.pieces`, `ingest.end_to_end`, `ingest.outcome`, `ingest.queue_depth`; luego el worker.

## Tramo 4: embedder (P7)

- [ ] Tests primero de `embedder.batch`, `embedder.batch_tokens`, `embedder.wait`, `embedder.load`; luego el canal de inferencia y la carga.

## Tramo 5: docs (P8)

- [ ] Tests primero de `docs.refresh.duration`, `docs.refresh.files`, `docs.search.inline_wait`; luego el refresco y `search_docs`.

## Tramo 6: eventos (P9, D5)

- [ ] Test primero: un `metrics.db` del esquema 1 sembrado sube al 2 con sus filas.
- [ ] La tabla `event`, su poda y su escritura con el minuto.
- [ ] Un test y un evento por tipo.
- [ ] `clawg metrics report --events`.

## Tramo 7: cierre

- [ ] P10 en `binary.rs`.
- [ ] `.claude/CLAUDE.md` con lo que quede.
- [ ] Verificación: suite y clippy en Linux; el CI en verde en Linux y Windows; el sobrecoste con el modelo real cuando haya autorización.
- [ ] Archive: `specs/metrics`; `git mv` a `changes/archive/`; `state.yaml`; fila 73 del `README.md` de openspec; subir el minor.
